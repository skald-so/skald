//! `skald sign-out`: ends the machine's session on the server and deletes its session token. The
//! device's keys, its certificate and the rest of the local state stay for its next sign-in.

use std::path::Path;
use std::time::Duration;

use skald_proto::VersionRange;

use crate::connection::{self, Connection};
use crate::devices::text;
use crate::secrets::{self, Secrets};
use crate::sign_in::{self, SESSION_SERVER, SESSION_TOKEN};
use crate::state::{self, State};

/// What `skald sign-out` says when the machine has no session.
const NOT_SIGNED_IN: &str = "This machine is not signed in.";

/// Signs out the machine whose state is in `dir`, its secrets in `storage`.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    // Without its local state, a machine has never signed in: it gets none.
    if !dir.join(state::FILE).is_file() {
        println!("{NOT_SIGNED_IN}");
        return Ok(());
    }
    // The key storage's entry is named by the directory's absolute path.
    let dir = &dir
        .canonicalize()
        .map_err(|err| format!("cannot use the state directory {}: {err}", dir.display()))?;
    let state = State::open(dir).map_err(text)?;
    let secrets = Secrets::new(dir, storage);
    let mut stored = secrets.load().map_err(text)?;
    // The token goes only to the server it came from, kept with it.
    let (Some(token), Some(server)) = (
        stored.get(SESSION_TOKEN).cloned(),
        stored.get(SESSION_SERVER).cloned(),
    ) else {
        sign_in::forget(&state, &secrets, &mut stored).map_err(text)?;
        println!("{NOT_SIGNED_IN}");
        return Ok(());
    };

    let mut connection = Connection::new(&server, speaks, timeout).map_err(text)?;
    match connection.delete_as("/session", &token) {
        // A session the server refuses has ended already.
        Ok(()) | Err(connection::Error::Refused { status: 401, .. }) => {}
        Err(err) => {
            return Err(format!(
                "{err}; the session did not end, so this machine is still signed in: run skald \
                 sign-out again"
            ));
        }
    }
    sign_in::forget(&state, &secrets, &mut stored).map_err(text)?;
    println!("Signed out of {server}. This device stays for its next sign-in.");
    Ok(())
}
