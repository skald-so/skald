//! Account deletion (`deleteAccount` in the OpenAPI spec).
//!
//! `skald delete-account`, only by its person at a terminal: it shows what the server says a
//! deletion deletes and what blocks it, and stops while a space blocks it. Otherwise it says what
//! goes at once and what 30 days later, and takes the account's username, typed at the terminal.
//! With a sign-in older than the server allows, it signs in again by device flow first and deletes
//! with that session. Once the server has deleted the account, it deletes the secrets, the device's
//! keys among them, from key storage and the state directory, and the local state. A run that sent
//! the deletion and got no answer leaves a mark ([`SENT`]): the next one that finds the session
//! refused takes the account as deleted and finishes.
//!
//! On the account's other devices, the helper notes a connection the server closes because the
//! account is deleted ([`mailbox::note_closing`]) and marks the state ([`REPORTED`]); then each
//! command that would reach the server says so and sends nothing ([`reported`]).

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
use crate::state::{self, State};
use crate::{helper, lock, output};

/// What `skald delete-account` says through a pipe.
pub const NOT_AT_TERMINAL: &str = "skald delete-account takes the account's username only from \
                                   its person typing it at a terminal, never through a pipe; \
                                   nothing was sent";

/// What `skald delete-account` says when the server refuses the session of a machine that sent no
/// deletion.
const NO_SESSION: &str = "the server no longer takes this machine's session: run skald sign-in, \
                          then skald delete-account again";

/// The mark of a deletion this machine sent whose answer it did not get, in the state directory.
const SENT: &str = "deletion-sent";
/// The mark of the server's report that the machine's account is deleted, in the state directory.
const REPORTED: &str = "account-deleted";

/// The problems of a sign-in older than the server allows and of a space that blocks the deletion.
const SIGN_IN_TOO_OLD: &str = "https://skald.so/problems/sign-in-too-old";
const DELETION_BLOCKED: &str = "https://skald.so/problems/deletion-blocked";

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
    if !dir.join(state::FILE).is_file() {
        return Err(devices::NOT_SIGNED_IN.to_owned());
    }
    helper::stop(dir, timeout)?;
    let lock = lock::take(dir, false, timeout)?;
    let (state, server, token, mut stored, secrets) =
        devices::signed_in_with_secrets(dir, storage)?;
    let mut connection = devices::to_session(&server, speaks, timeout)?;
    let sent = dir.join(SENT);
    let account = match account_of(&mut connection, &token) {
        Ok(account) => account,
        // The deletion this machine sent took its session with it.
        Err(connection::Error::Refused { status: 401, .. }) if sent.is_file() => {
            return erase(dir, (state, secrets), lock);
        }
        Err(connection::Error::Refused { status: 401, .. }) => {
            return Err(NO_SESSION.to_owned());
        }
        Err(err) => return Err(err.to_string()),
    };
    let _ = fs::remove_file(&sent);
    let preview = preview(&mut connection, &token)?;
    if !preview.blocks.is_empty() {
        return Err(blocked(&preview.blocks));
    }
    let personal: Vec<String> = (state.spaces().map_err(text)?.into_iter())
        .filter(|space| space.personal)
        .map(|space| space.id.to_string())
        .collect();
    let username = printable(&account.username);
    println!("Deleting the account {username} deletes at once:");
    println!("- its sessions and sign-in methods, and its devices, this one's keys and state too;");
    if !preview.deletes.is_empty() {
        println!("- the spaces only it is in:");
        for listed in &preview.deletes {
            let name = printable(&listed.name);
            if personal.contains(&listed.space) {
                println!("  {name}, the personal space of {username}");
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
        &format!("Type the account's username, {username}, to delete it"),
        "",
    )?;
    if typed.as_deref().map(str::trim) != Some(account.username.as_str()) {
        return Err(format!("that is not {username}: nothing is deleted"));
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
        Ok(()) => erase(dir, (state, secrets), lock),
        Err(err) if !err.reached() => Err(format!(
            "{err}; the account may be deleted already: run skald delete-account again to finish"
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
    let token = sign_in::device_flow(connection, dir).map_err(|err| {
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

/// Deletes what the machine whose state is in `dir` keeps of its deleted account: the `secrets`,
/// from key storage and the state directory, and every other file there, the local `state`
/// closed first and the `lock` file last; then says the account is deleted.
fn erase(
    dir: &Path,
    (state, secrets): (State, Secrets),
    lock: Option<lock::Lock>,
) -> Result<(), String> {
    let username = state
        .session()
        .ok()
        .flatten()
        .and_then(|session| session.username);
    drop(state);
    secrets.erase().map_err(text)?;
    let failed =
        |err: io::Error| format!("cannot empty the state directory {}: {err}", dir.display());
    for entry in fs::read_dir(dir).map_err(failed)? {
        let path = entry.map_err(failed)?.path();
        if path.file_name().is_some_and(|name| name == lock::FILE) {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        }
        .map_err(failed)?;
    }
    drop(lock);
    let _ = fs::remove_file(dir.join(lock::FILE));
    match username {
        Some(username) => println!(
            "The account {} is deleted, and this machine keeps nothing of it.",
            printable(&username)
        ),
        None => println!("The account is deleted, and this machine keeps nothing of it."),
    }
    Ok(())
}

/// Whether the server reported the account of the machine whose state is in `dir` deleted.
pub fn reported(dir: &Path) -> bool {
    dir.join(REPORTED).is_file()
}

/// What a run on a device of a deleted account says, the state directory being `dir`.
pub fn said(dir: &Path) -> String {
    format!(
        "the server reports this machine's account deleted, so it sends and receives nothing: to \
         use this machine with another account, remove its state directory {}",
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
    /// Whether the server closed a connection of the helper of the state in `dir` because the
    /// account is deleted: then it marks the state and says so, once. Otherwise, if `idle` and no
    /// connection is kept, it opens the device's connection with `settings`, at most once in
    /// [`WATCH_AGAIN`].
    pub fn reported(&mut self, dir: &Path, settings: Settings, idle: bool) -> bool {
        if mailbox::account_deleted() {
            let _ = fs::write(dir.join(REPORTED), b"");
            let line = format!("skald: {}\n", said(dir));
            let _ = output::direct(output::Stream::Err, line.as_bytes());
            return true;
        }
        let due = self.next.is_none_or(|next| Instant::now() >= next);
        if idle && due && mailbox::kept_transport().is_none() && !reported(dir) {
            self.next = Some(Instant::now() + WATCH_AGAIN);
            watch(dir, settings);
        }
        false
    }
}

/// Opens the connection of the certified `client` device of the state in `dir`, with `settings`,
/// and keeps it ([`Mailbox`]); does nothing without one, or when it does not open.
fn watch(dir: &Path, (storage, speaks, timeout): Settings) {
    let Ok(Some(mut follower)) = Follower::unchecked(dir, storage, speaks, timeout) else {
        return;
    };
    if follower.cert.body().kind != DeviceKind::Client {
        return;
    }
    if let Ok(keys) = follower.keys() {
        // Dropped, it is kept for the helper's next operation.
        let _ = Mailbox::open(&mut follower, &keys);
    }
}
