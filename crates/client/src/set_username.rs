//! `skald set-username`: sets the username of the account of the machine's session, on the server
//! of that session, through `PUT /account/username`. A username the server refuses fails with the
//! server's reason, and the account keeps its old one.

use std::path::Path;
use std::time::Duration;

use skald_proto::VersionRange;

use crate::connection::{self, Connection, Method, printable};
use crate::devices::{signed_in, text};
use crate::secrets;

/// Sets `username` as the username of the account the machine whose state is in `dir` signed in
/// to, its secrets in `storage`, and records it in the local state.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
    username: &str,
) -> Result<(), String> {
    let (state, server, token) = signed_in(dir, storage)?;

    let mut connection = Connection::new(&server, speaks, timeout).map_err(text)?;
    let body = serde_json::json!({ "username": username }).to_string();
    match connection.send_json_as(Method::Put, "/account/username", &token, body.as_bytes()) {
        Ok(_) => {}
        Err(connection::Error::Refused {
            status: 400 | 409,
            detail,
        }) => {
            return Err(refused(username, &detail));
        }
        Err(err) => return Err(err.to_string()),
    }
    state.set_username(username).map_err(text)?;
    println!(
        "The account's username on {server} is now {}.",
        printable(username)
    );
    Ok(())
}

/// What `skald set-username` says when the server refuses `username` for the reason `detail`.
fn refused(username: &str, detail: &str) -> String {
    format!(
        "the server refused the username \"{}\": {}; the account keeps its username",
        printable(username),
        printable(detail)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_gives_the_username_and_the_servers_reason_without_control_characters() {
        assert_eq!(
            refused("root\u{1b}", "The username root is reserved.\u{7}"),
            "the server refused the username \"root\\u{1b}\": The username root is reserved.\\u{7}; \
             the account keeps its username"
        );
    }
}
