//! Account deletion (`deleteAccount` in the OpenAPI spec).
//!
//! `skald delete-account`, only by its person at a terminal: it shows what the server says a
//! deletion deletes and what blocks it, and stops while a space blocks it. Otherwise it says what
//! goes at once and what 30 days later, and takes the account's username, typed at the terminal.
//! With a sign-in older than the server allows, it signs in again by device flow first and deletes
//! with that session. Once the server has deleted the account, it deletes the secrets, the device's
//! keys among them, from key storage and the state directory, and the local state, marked deleted
//! ([`REPORTED`]) until it is gone. A run that sent the deletion leaves a mark ([`SENT`]) until the
//! server itself refused it: the next run, if the server refuses the session and says the account
//! is deleted, finishes; on a device without its certificate, which cannot ask, it finishes once
//! its person types the username again.
//!
//! A device learns that its account is deleted as the server says so: the helper, from a connection
//! the server closes ([`mailbox::note_closing`]); any run whose session the server refused, from a
//! session it asks by the device's key ([`learned`]). Either marks the state ([`REPORTED`]); then
//! each command that would reach the server says so and sends nothing ([`reported`]), and `skald
//! delete-account` deletes the local state.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;
use skald_proto::{DeviceKind, VersionRange};

use crate::claude_socket::Settings;
use crate::connection::{self, Connection, printable};
use crate::devices::{self, text};
use crate::mailbox::{self, Mailbox};
use crate::person::Person;
use crate::secrets::{self, Secrets};
use crate::sign_in::{self, SESSION_TOKEN};
use crate::spaces::Follower;
use crate::state::{self, Link, State};
use crate::{device, helper, lock, output};

/// What `skald delete-account` says through a pipe.
pub const NOT_AT_TERMINAL: &str = "skald delete-account takes the account's username only from \
                                   its person typing it at a terminal, never through a pipe; \
                                   nothing was sent";

/// What `skald delete-account` says when the server refuses the session of a machine that sent no
/// deletion.
const NO_SESSION: &str = "the server no longer takes this machine's session: run skald sign-in, \
                          then skald delete-account again";

/// What `skald delete-account` says when the server refuses the session of a machine without its
/// certificate whose deletion got no answer of the server's.
const MAY_HAVE_GONE: &str = "The server no longer takes this machine's session, and the deletion it \
                             sent may have gone through; without its certificate, this machine \
                             cannot ask the server.";

/// The mark of a deletion this machine sent whose answer it did not get, in the state directory.
const SENT: &str = "deletion-sent";
/// The mark of the server's report that the machine's account is deleted, in the state directory.
const REPORTED: &str = "account-deleted";

/// Where the types of the server's own problems start.
const PROBLEMS: &str = "https://skald.so/problems/";
/// The problems of a sign-in older than the server allows, of a space that blocks the deletion and
/// of a device of a deleted account.
const SIGN_IN_TOO_OLD: &str = "https://skald.so/problems/sign-in-too-old";
const DELETION_BLOCKED: &str = "https://skald.so/problems/deletion-blocked";
const ACCOUNT_DELETED: &str = "https://skald.so/problems/account-deleted";

/// `AccountDeletion` in the OpenAPI spec.
#[derive(Deserialize)]
struct Preview {
    deletes: Vec<Listed>,
    blocks: Vec<Listed>,
}

/// `DeletionSpace` in the OpenAPI spec.
#[derive(Deserialize)]
struct Listed {
    space: String,
    name: String,
}

/// The session's account, checked with the server.
struct Account {
    id: String,
    username: String,
}

/// `skald delete-account` on the machine whose state is in `dir`, its secrets in `storage`, run by
/// its person at a terminal.
pub fn run(
    dir: &Path,
    storage: secrets::Storage,
    speaks: VersionRange,
    timeout: Duration,
) -> Result<(), String> {
    let sent = dir.join(SENT);
    if !reported(dir) && !dir.join(state::FILE).is_file() {
        return Err(devices::NOT_SIGNED_IN.to_owned());
    }
    helper::stop(dir, timeout)?;
    let lock = lock::take(dir, false, timeout)?;
    // Reported by the server, or by a run that stopped while it deleted the local state.
    if reported(dir) {
        return erase_deleted(dir, storage, None, lock);
    }
    let (state, server, token, mut stored, secrets) =
        devices::signed_in_with_secrets(dir, storage)?;
    let username = state.session().ok().flatten().and_then(|it| it.username);
    let mut connection = devices::to_session(&server, speaks, timeout)?;
    let account = match account_of(&mut connection, &token) {
        Ok(account) => account,
        Err(connection::Error::Refused { status: 401, .. }) => {
            let deleted = by_key(&mut connection, &state, &stored)?;
            drop(state);
            return match deleted {
                Some(true) => erase_deleted(dir, storage, username, lock),
                // A device without its certificate cannot ask whether a deletion it sent went
                // through: its person says.
                None if sent.is_file() => confirmed(dir, storage, username, lock),
                _ => {
                    let _ = fs::remove_file(&sent);
                    Err(NO_SESSION.to_owned())
                }
            };
        }
        Err(err) => return Err(err.to_string()),
    };
    // The account lives: a deletion an earlier run sent did not reach the server.
    let _ = fs::remove_file(&sent);
    let preview = preview(&mut connection, &token)?;
    if !preview.blocks.is_empty() {
        return Err(blocked(&preview.blocks));
    }
    let personal: Vec<String> = (state.spaces().map_err(text)?.into_iter())
        .filter(|space| space.personal)
        .map(|space| space.id.to_string())
        .collect();
    drop(state);
    let handle = printable(&account.username);
    println!("Deleting the account {handle} deletes at once:");
    println!("- its sessions and sign-in methods, and its devices, this one's keys and state too;");
    if !preview.deletes.is_empty() {
        println!("- the spaces only it is in:");
        for listed in &preview.deletes {
            let name = printable(&listed.name);
            if personal.contains(&listed.space) {
                println!("  {name}, the personal space of {handle}");
            } else {
                println!("  {name}");
            }
        }
    }
    println!(
        "30 days later, the server erases the rest of its data, but for what the spaces it shares \
         keep: the roster revisions that name it, its devices' certificates and the messages they \
         sent. Nothing undoes a deletion."
    );
    let typed = Person::Terminal.enter(
        &format!("Type the account's username, {handle}, to delete it"),
        "",
    )?;
    if typed.as_deref().map(str::trim) != Some(account.username.as_str()) {
        return Err(format!("that is not {handle}: nothing is deleted"));
    }

    fs::write(&sent, b"").map_err(|err| format!("cannot mark the deletion as sent: {err}"))?;
    let deleted = match connection.delete_as("/account", &token) {
        Err(connection::Error::Refused { kind, .. }) if kind == SIGN_IN_TOO_OLD => {
            let fresh = signed_in_again(&mut connection, dir, &account, (&secrets, &mut stored));
            let fresh = fresh.inspect_err(|_| {
                let _ = fs::remove_file(&sent);
            })?;
            // The older session goes: the machine keeps the new one.
            let _ = connection.delete_as("/session", &token);
            connection.delete_as("/account", &fresh)
        }
        deleted => deleted,
    };
    match deleted {
        Ok(()) => erase_deleted(dir, storage, Some(account.username), lock),
        // An answer that is not the server's, or none, may follow a deletion.
        Err(err) if !server_refused(&err) => Err(format!(
            "{err}; the deletion may have gone through: run skald delete-account again to finish"
        )),
        Err(err) => {
            let _ = fs::remove_file(&sent);
            match err {
                connection::Error::Refused { kind, .. } if kind == DELETION_BLOCKED => {
                    Err(blocked(&preview_of(&mut connection, &secrets)?.blocks))
                }
                err => Err(format!("{err}; nothing is deleted")),
            }
        }
    }
}

/// Whether `err` is the server's own refusal: a problem of its own type.
fn server_refused(err: &connection::Error) -> bool {
    matches!(err, connection::Error::Refused { kind, .. } if kind.starts_with(PROBLEMS))
}

/// Whether the server says the account of the device of `state`, its secrets `stored`, is deleted,
/// asked on `connection` by a session by the device's key, which ends at once if given; `None`
/// without a certified device and its keys, which asks nothing.
fn by_key(
    connection: &mut Connection,
    state: &State,
    stored: &secrets::Map,
) -> Result<Option<bool>, String> {
    let Some(device) = state.device().map_err(text)? else {
        return Ok(None);
    };
    let Link::Certified { cert, .. } = &device.link else {
        return Ok(None);
    };
    let Some(keys) = device::known_keys(stored, Some(&device)).map_err(text)? else {
        return Ok(None);
    };
    match connection.device_key_session(keys.id(), keys.signing(), cert) {
        Ok(token) => {
            let _ = connection.delete_as("/session", &token);
            Ok(Some(false))
        }
        Err(connection::Error::Refused { kind, .. }) if kind == ACCOUNT_DELETED => Ok(Some(true)),
        Err(err) if server_refused(&err) => Ok(Some(false)),
        Err(err) => Err(err.to_string()),
    }
}

/// After a run on the machine whose state is in `dir` whose session the server refused, with
/// `settings`: what it says if the server says, asked by the device's key, that the account is
/// deleted, which marks the state.
pub fn learned(dir: &Path, (storage, speaks, timeout): Settings) -> Option<String> {
    let (state, server, _, stored, _) = devices::signed_in_with_secrets(dir, storage).ok()?;
    let mut connection = devices::to_session(&server, speaks, timeout).ok()?;
    if by_key(&mut connection, &state, &stored) != Ok(Some(true)) {
        return None;
    }
    mark(dir);
    Some(said(dir))
}

/// Marks the state in `dir` as of an account the server reports deleted.
fn mark(dir: &Path) {
    let _ = fs::write(dir.join(REPORTED), b"");
}

/// The account of the session `token`, as the server of `connection` gives it.
fn account_of(connection: &mut Connection, token: &str) -> Result<Account, connection::Error> {
    let body = connection.get_as("/session", token)?;
    let account: sign_in::Account = serde_json::from_slice(&body)
        .map_err(|_| connection::Error::Unexpected("a session without its account".to_owned()))?;
    Ok(Account {
        id: account.account,
        username: account.username,
    })
}

/// What the server says a deletion of the account of the session `token` deletes and what blocks
/// it.
fn preview(connection: &mut Connection, token: &str) -> Result<Preview, String> {
    let body = connection
        .get_as("/account/deletion", token)
        .map_err(|err| err.to_string())?;
    serde_json::from_slice(&body).map_err(|_| {
        "unexpected answer from the server: what a deletion deletes, without its spaces".to_owned()
    })
}

/// [`preview`] with the session `secrets` keep.
fn preview_of(connection: &mut Connection, secrets: &Secrets) -> Result<Preview, String> {
    let stored = secrets.load().map_err(text)?;
    let token = stored
        .get(SESSION_TOKEN)
        .ok_or_else(|| devices::NOT_SIGNED_IN.to_owned())?;
    preview(connection, token)
}

/// Says the spaces of `blocks`, each with the command that hands over its admin rights; gives why
/// nothing is deleted.
fn blocked(blocks: &[Listed]) -> String {
    println!("These spaces have other members and no admin but this account:");
    for listed in blocks {
        let name = printable(&listed.name);
        println!(
            "  {name}: hand its admin rights to a member first: skald hand-over-admin {name} \
             <handle>"
        );
    }
    "nothing is deleted while a space has no other admin".to_owned()
}

/// Signs in again by device flow on the server of `connection`, the verification page's file in
/// `dir`, as the person of `account`: keeps the new session in `stored`, the `secrets`, and gives
/// its token. A session of another account ends.
fn signed_in_again(
    connection: &mut Connection,
    dir: &Path,
    account: &Account,
    (secrets, stored): (&Secrets, &mut secrets::Map),
) -> Result<String, String> {
    println!("The session's sign-in is too old to delete the account: sign in again.");
    let token = sign_in::device_flow(connection, dir, None).map_err(|err| {
        match err {
            sign_in::Error::Denied => "the sign-in was denied in the browser: nothing is deleted",
            sign_in::Error::Expired => "the sign-in code expired: nothing is deleted",
            err => return format!("{err}; nothing is deleted"),
        }
        .to_owned()
    })?;
    let signed_in = account_of(connection, &token).map_err(|err| err.to_string())?;
    if signed_in.id != account.id {
        let _ = connection.delete_as("/session", &token);
        return Err(format!(
            "account {}, which is not this device's, signed in: nothing is deleted",
            printable(&signed_in.username)
        ));
    }
    stored.insert(SESSION_TOKEN.to_owned(), token.clone());
    secrets.save(stored).map_err(text)?;
    Ok(token)
}

/// Erases what the machine whose state is in `dir`, its secrets in `storage`, keeps of its deleted
/// account ([`erase`]); says the account, `username` if known, is deleted.
fn erase_deleted(
    dir: &Path,
    storage: secrets::Storage,
    username: Option<String>,
    lock: Option<lock::Lock>,
) -> Result<(), String> {
    erase(dir, storage, lock)?;
    match username {
        Some(username) => println!(
            "The account {} is deleted, and this machine keeps nothing of it.",
            printable(&username)
        ),
        None => println!("The account is deleted, and this machine keeps nothing of it."),
    }
    Ok(())
}

/// On the machine whose state is in `dir`, its secrets in `storage`, which has no certificate to ask
/// whether the deletion it sent went through: erases what it keeps of the account ([`erase`]) once
/// its person types the account's username, `username`, or `erase` if unknown.
fn confirmed(
    dir: &Path,
    storage: secrets::Storage,
    username: Option<String>,
    lock: Option<lock::Lock>,
) -> Result<(), String> {
    println!("{MAY_HAVE_GONE}");
    let (expected, what) = match &username {
        Some(username) => (
            username.as_str(),
            format!("the account's username, {},", printable(username)),
        ),
        None => ("erase", "erase".to_owned()),
    };
    let typed = Person::Terminal.enter(
        &format!("If it went through, type {what} to erase this machine's keys and state"),
        "",
    )?;
    if typed.as_deref().map(str::trim) != Some(expected) {
        return Err(format!(
            "nothing is erased: this machine keeps its keys and its state directory {}",
            dir.display()
        ));
    }
    erase(dir, storage, lock)?;
    println!("This machine keeps nothing of the account any more.");
    Ok(())
}

/// Deletes what the machine whose state is in `dir`, its secrets in `storage`, keeps of its
/// account, the local state closed: marks the state as of a deleted account first, so that the next
/// run finishes what a run that stops on the way leaves; then deletes the secrets, from key storage
/// and the state directory, every other file there, the marks, and the `lock` file last.
fn erase(dir: &Path, storage: secrets::Storage, lock: Option<lock::Lock>) -> Result<(), String> {
    let failed =
        |err: io::Error| format!("cannot empty the state directory {}: {err}", dir.display());
    fs::write(dir.join(REPORTED), b"").map_err(failed)?;
    // The key storage's entry is named by the directory's canonical path, as at sign-in.
    let canonical = dir.canonicalize().map_err(failed)?;
    Secrets::new(&canonical, storage).erase().map_err(text)?;
    let kept = [lock::FILE, SENT, REPORTED];
    for entry in fs::read_dir(dir).map_err(failed)? {
        let path = entry.map_err(failed)?.path();
        if path
            .file_name()
            .is_some_and(|name| kept.iter().any(|kept| name == *kept))
        {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        }
        .map_err(failed)?;
    }
    for mark in [SENT, REPORTED] {
        match fs::remove_file(dir.join(mark)) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(failed(err)),
            _ => {}
        }
    }
    drop(lock);
    let _ = fs::remove_file(dir.join(lock::FILE));
    Ok(())
}

/// Whether the server reported the account of the machine whose state is in `dir` deleted.
pub fn reported(dir: &Path) -> bool {
    dir.join(REPORTED).is_file()
}

/// What a run on a device of a deleted account says, the state directory being `dir`.
pub fn said(dir: &Path) -> String {
    format!(
        "the server reports this machine's account deleted, so it sends and receives nothing: run \
         skald delete-account to erase its keys and its state directory {}",
        dir.display()
    )
}

/// How often the helper opens the device's connection again while it keeps none, to learn of the
/// account's deletion.
const WATCH_AGAIN: Duration = Duration::from_secs(60);

/// The helper's watch for its account's deletion: while it waits idle it keeps the device's
/// connection open, which the server closes once the account is deleted.
#[derive(Default)]
pub struct Watch {
    next: Option<Instant>,
}

impl Watch {
    /// Whether the server reported the account of the helper of the state in `dir` deleted: then it
    /// marks the state and says so, once. It learns so from a connection the server closed because
    /// the account is deleted; or, if `idle` and no connection is kept, from opening the device's
    /// connection with `settings`, at most once in [`WATCH_AGAIN`], which a refused session makes
    /// it ask ([`learned`]).
    pub fn reported(&mut self, dir: &Path, settings: Settings, idle: bool) -> bool {
        let mut deleted = mailbox::account_deleted();
        let due = self.next.is_none_or(|next| Instant::now() >= next);
        if !deleted && idle && due && mailbox::kept_transport().is_none() && !reported(dir) {
            self.next = Some(Instant::now() + WATCH_AGAIN);
            deleted = watch(dir, settings);
        }
        if deleted {
            mark(dir);
            let line = format!("skald: {}\n", said(dir));
            let _ = output::direct(output::Stream::Err, line.as_bytes());
        }
        deleted
    }
}

/// Opens the connection of the certified `client` device of the state in `dir`, with `settings`,
/// and keeps it ([`Mailbox`]); does nothing without one. Gives whether the server reports the
/// account deleted, asked once it refused the session.
fn watch(dir: &Path, settings: Settings) -> bool {
    let (storage, speaks, timeout) = settings;
    let Ok(Some(mut follower)) = Follower::unchecked(dir, storage, speaks, timeout) else {
        return false;
    };
    if follower.cert.body().kind != DeviceKind::Client {
        return false;
    }
    let Ok(keys) = follower.keys() else {
        return false;
    };
    connection::forget_refused();
    // Dropped, it is kept for the helper's next operation.
    let opened = Mailbox::open(&mut follower, &keys).is_ok();
    drop(follower);
    !opened && connection::session_refused() && learned(dir, settings).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    /// The state directory of a machine without its secrets, as a run that stopped while it deleted
    /// the local state leaves it: a database, a subdirectory, its lock file and `marks`.
    fn stopped_in_erase(name: &str, marks: &[&str]) -> TestDir {
        let dir = TestDir::new(name);
        fs::write(dir.join(state::FILE), b"the account's local state").unwrap();
        fs::create_dir(dir.join("helper")).unwrap();
        fs::write(dir.join("helper").join("file"), b"").unwrap();
        fs::write(dir.join(lock::FILE), b"").unwrap();
        for mark in marks {
            fs::write(dir.join(mark), b"").unwrap();
        }
        dir
    }

    #[test]
    fn a_marked_run_without_secrets_finishes_the_erase() {
        for marks in [&[REPORTED][..], &[SENT, REPORTED]] {
            let dir = stopped_in_erase("deletion-finishes", marks);

            run(
                &dir,
                secrets::Storage::File,
                VersionRange::default(),
                Duration::from_secs(1),
            )
            .unwrap();

            let left: Vec<_> = fs::read_dir(&*dir).unwrap().collect();
            assert!(left.is_empty(), "{marks:?}: {left:?}");
        }
    }

    #[test]
    fn without_the_servers_report_a_run_without_a_session_deletes_nothing() {
        for marks in [&[][..], &[SENT]] {
            let dir = stopped_in_erase("deletion-unreported", marks);

            let refused = run(
                &dir,
                secrets::Storage::File,
                VersionRange::default(),
                Duration::from_secs(1),
            );

            assert!(refused.is_err(), "{marks:?}: {refused:?}");
            assert!(dir.join(state::FILE).is_file(), "{marks:?}");
            assert!(dir.join("helper").join("file").is_file(), "{marks:?}");
        }
    }

    #[test]
    fn only_the_servers_own_problems_are_its_refusals() {
        let refused = |kind: &str| connection::Error::Refused {
            status: 403,
            kind: kind.to_owned(),
            detail: String::new(),
            task: None,
        };
        assert!(server_refused(&refused(DELETION_BLOCKED)));
        assert!(!server_refused(&refused("about:blank")));
        assert!(!server_refused(&connection::Error::Unexpected(
            "HTTP 502".to_owned()
        )));
        assert!(!server_refused(&connection::Error::Timeout(
            Duration::from_secs(1)
        )));
    }
}
