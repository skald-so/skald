//! The recovery key of the account's first device (`docs/protocol/membership.md`, "Account"):
//! after a "yes" on an account without a master key, `skald sign-in` also creates the recovery key,
//! the account record and the master backup, gives them to the server with the master key, and once
//! the server takes them shows the key once at a terminal, then deletes it. Run as an agent, with
//! its output not a terminal, or killed before the deletion, the key waits for the person's next
//! `skald sign-in` at a terminal; meanwhile other commands print one line naming that command and
//! never the key. Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on
//! PostgreSQL, behind a recording relay, with a test playing the person in headless Chrome; the
//! person's terminal is a pseudo-terminal, or a pseudo-console on Windows.
//!
//! The CLI, as these tests use it, beyond the device keys' tests:
//! - at a terminal, `skald sign-in` shows the recovery key in its shown form, 13 groups of four
//!   base32 characters joined by `-`, with what it is for, and deletes it once the person presses
//!   Enter: until then a kill leaves it waiting;
//! - while it waits, every other run prints one line that holds `recovery key` and the full command
//!   that shows it, `skald sign-in --server <the device's server>`;
//! - `skald sign-in` on an account whose master key has no account record says it has `no recovery
//!   key`.

#[allow(dead_code)]
mod at_terminal;
mod backup;
#[path = "../../../server/tests/common/browser.rs"]
#[allow(dead_code)]
mod browser;
#[path = "../sign_in/cloud.rs"]
#[allow(dead_code)]
mod cloud;
#[path = "../common/mod.rs"]
#[allow(dead_code)]
mod common;
mod grants;
#[path = "../device_keys/keys.rs"]
#[allow(dead_code)]
mod keys;
mod lost;
#[path = "../sign_in/machine.rs"]
#[allow(dead_code)]
mod machine;
#[path = "../../../crypto/tests/common/master_backup.rs"]
#[allow(dead_code)]
mod master_backup;
mod others;
#[path = "../sign_in/relay.rs"]
#[allow(dead_code)]
mod relay;
mod shown;
mod system;
mod terminal;

pub use at_terminal::{AGENT, approve, as_agent, first_at_terminal, keys_in, shown_key, sign_in};
use browser::{APPROVE, Browser};
use cloud::{Cloud, unb64};
use machine::{FIRST_DEVICE, Machine, YES};
use serde_json::Value;
use skald_proto::AccountId;
use terminal::{Shown, Terminal};

/// What a machine that waits for linking says, at a terminal, before it shows its link code.
pub const LINK_CODE: &str = "type this code there";

/// Lets `run`, of a machine that waits for linking, go on until it shows its link code, then ends
/// it, its person linking nothing now; gives what it printed.
#[track_caller]
pub fn until_link_code(mut run: Terminal) -> Shown {
    run.wait_to_show(LINK_CODE);
    run.kill()
}

/// A server whose codes live for 2 minutes and poll every second, and a browser for the person.
pub fn setup() -> (Cloud, Browser) {
    (Cloud::start(120, 1), Browser::launch())
}

/// Whether `text` holds the recovery key `key` in any case, with or without `-` and spaces.
pub fn holds_key(text: &str, key: &[u8; 32]) -> bool {
    let squeeze = |text: &str| -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .flat_map(char::to_uppercase)
            .collect()
    };
    squeeze(text).contains(&squeeze(&backup::shown_form(key)))
}

/// Whether `bytes`, what a machine or a server keeps or sends, hold the recovery key `key`: as
/// bytes in any of [`keys`]'s encodings, or as text in any of [`holds_key`]'s forms.
pub fn bytes_hold_key(bytes: &[u8], key: &[u8; 32]) -> bool {
    keys::holds(bytes, key) || holds_key(&String::from_utf8_lossy(bytes), key)
}

/// The lines of `text` that say a recovery key waits: those that hold `recovery key`, in any
/// case, and the command `skald sign-in --server <cloud>`.
pub fn waiting_lines<'t>(cloud: &Cloud, text: &'t str) -> Vec<&'t str> {
    let command = format!("skald sign-in --server {}", cloud.url);
    text.lines()
        .filter(|line| line.contains(&command) && line.to_lowercase().contains("recovery key"))
        .collect()
}

/// Fails unless `text`, what a run printed while the recovery key waits, says so in one line and
/// shows no recovery key.
#[track_caller]
pub fn assert_waits(cloud: &Cloud, what: &str, text: &str) {
    assert_eq!(
        waiting_lines(cloud, text).len(),
        1,
        "{what}: one line on the waiting key: {text}"
    );
    assert_eq!(
        keys_in(text),
        Vec::<String>::new(),
        "{what} shows a key: {text}"
    );
}

/// Fails unless `text` neither shows a recovery key nor says one waits.
#[track_caller]
pub fn assert_quiet(cloud: &Cloud, what: &str, text: &str) {
    assert_eq!(
        waiting_lines(cloud, text),
        Vec::<&str>::new(),
        "{what} says a key waits: {text}"
    );
    assert_eq!(
        keys_in(text),
        Vec::<String>::new(),
        "{what} shows a key: {text}"
    );
}

/// The person reads the recovery key `run` shows, then presses Enter; gives the key's bytes and
/// what the run printed. Fails unless the run succeeds and shows that one key.
#[track_caller]
pub fn written_down(run: Terminal) -> ([u8; 32], Shown) {
    let mut run = run;
    let key = shown_key(&mut run);
    run.enter();
    let shown = run.finish();
    assert!(shown.success, "{shown:?}");
    assert_eq!(keys_in(&shown.text), vec![key.clone()], "{shown:?}");
    (backup::key_bytes(&key), shown)
}

/// What `GET /account/master` gives the session `token`: the master key, the account record and
/// the master backup, each if it has one.
pub struct Held {
    pub master: Option<[u8; 32]>,
    pub record: Option<Vec<u8>>,
    pub backup: Option<Vec<u8>>,
}

pub fn held(cloud: &Cloud, token: &str) -> Held {
    let (status, body) = cloud.get_as("/account/master", token);
    assert_eq!(status, 200, "GET /account/master: {body}");
    let bytes = |field: &str| match body.get(field).unwrap_or(&Value::Null) {
        Value::Null => None,
        Value::String(text) => Some(unb64(text)),
        other => panic!("{field} of GET /account/master: {other}"),
    };
    Held {
        master: bytes("master").map(|master| master.try_into().expect("a 32-byte master key")),
        record: bytes("record"),
        backup: bytes("backup"),
    }
}

/// Checks that the server holds, for the account of `token`, an account record and a master backup
/// that the recovery key `key` stands behind: the record, revision 1, names the account, the
/// master key the server holds and `machine` pinned, and the recovery HPKE public key of `key`, and
/// that master key signs it; the backup opens with `key` to that master key's private key. Gives
/// the master private key.
#[track_caller]
pub fn opens_the_backup(cloud: &Cloud, machine: &Machine, token: &str, key: &[u8; 32]) -> [u8; 32] {
    let account: AccountId = cloud.account_of(token);
    let held = held(cloud, token);
    let master = held.master.expect("the account's master key");
    assert!(
        keys::holds(&machine.state_bytes(), &master),
        "the machine did not pin the master key"
    );
    let record = backup::Record::of(&held.record.expect("the account record"));
    assert_eq!(record.account, account.as_bytes(), "the record's account");
    assert_eq!(record.master, master, "the record's master key");
    assert_eq!(
        record.recovery,
        backup::recovery_public(key),
        "the record's recovery key"
    );
    assert_eq!(record.revision, 1, "the record's revision");
    assert!(
        record.signed_by(&master),
        "the master key does not sign the record"
    );
    let seed = backup::open(&held.backup.expect("the master backup"), key, account)
        .expect("the backup opens with the recovery key");
    assert_eq!(backup::master_public(&seed), master, "the backup's key");
    seed
}

/// The person's `skald sign-in --server <cloud> --first-device yes` on `machine` at a terminal, once
/// signed in: gives the recovery key it shows, read as [`written_down`] reads it.
#[track_caller]
pub fn key_at_sign_in(cloud: &Cloud, machine: &Machine) -> [u8; 32] {
    let run = Terminal::start(machine, &sign_in(cloud, &[FIRST_DEVICE, YES]));
    written_down(run).0
}

/// Runs `skald sign-in --server <cloud> --first-device yes`, with `before` the arguments before it,
/// on `machine` at a terminal, once signed in; fails unless it succeeds, shows no recovery key and
/// says none waits. Gives what it printed.
#[track_caller]
pub fn quiet_sign_in(cloud: &Cloud, machine: &Machine, before: &[&str]) -> String {
    let args = [before, &sign_in(cloud, &[FIRST_DEVICE, YES])[..]].concat();
    let shown = Terminal::start(machine, &args).finish();
    assert!(shown.success, "{shown:?}");
    assert_quiet(cloud, "skald sign-in", &shown.text);
    shown.text
}
