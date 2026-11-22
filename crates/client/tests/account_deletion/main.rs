//! Deleting an account from the CLI: on a device of the account, at a terminal, `skald
//! delete-account` shows what the server says a deletion deletes and what blocks it, and stops
//! while a space blocks it, naming the command that hands over admin rights. Otherwise it says what
//! goes at once and what 30 days later, and takes the account's username, typed at the terminal,
//! never from a pipe or as an agent. With a sign-in older than the server's setting, it first signs
//! in again by device flow, as `skald sign-in` does, and deletes with that session. Once the server
//! has deleted the account, it deletes the device's keys from key storage, its secrets and its local
//! state; killed after sending, its next run finds the account deleted and finishes. The account's
//! other devices say the server reports the account deleted, naming the state directory to remove,
//! and send nothing (`architecture.md`, "Data model"; `deleteAccount` in the OpenAPI spec).
//!
//! Against a `skald-server` on PostgreSQL with GitHub sign-in through the stand-in, behind the
//! front and the recording relay of the tests of sending, with the person approving codes on the
//! verification page in headless Chrome. Machines keep their secrets in a file of their state
//! directory, never in the system's key storage, but for the test that only
//! `SKALD_TEST_REAL_KEY_STORAGE=1` runs, as the Windows job does with Credential Manager. A second
//! device of the account links with a link code, as in the tests of linking with one. Sign-ins are
//! made old by moving `sessions.created_at` back in the server's database, past the setting of 300
//! seconds the tests' server has; a device code is made to expire by moving its expiry back.
//!
//! The CLI, as these tests use it, besides what the tests of signing in, spaces, admin rights and
//! linking use:
//! - `skald delete-account`, on a device of the account, at a terminal:
//!   - while a space blocks the deletion, it names each such space on a line that holds the space's
//!     name and `hand-over-admin`, the command that hands over admin rights, and exits failing
//!     by itself, sending no request that deletes the account;
//!   - otherwise it names each space the deletion deletes on a line that holds the space's name,
//!     the account's personal space's line also holding `personal`; says what goes 30 days later,
//!     holding `30 days`; and asks for the account's username on a line holding `username`;
//!   - it deletes only once the typed username is the account's; another one deletes nothing, and
//!     the run exits failing;
//!   - with a sign-in older than the server's setting, it shows a device authorization's
//!     verification page and code, as `skald sign-in` does; a denied or expired code, or one
//!     approved by another account, deletes nothing, and the run exits failing, saying `denied` or
//!     `expire` for the first two;
//!   - once deleted, it says so on a line holding `account` and `deleted`, and exits with success;
//!     the machine's state directory then holds no SQLite database and no file holding the
//!     account's session tokens, its device's private keys, the account's or the device's ID or the
//!     username; with the system's key storage, the machine's entry there holds none of them;
//!   - killed once its request to delete the account was sent, before the answer, the machine keeps
//!     its local state; its next `skald delete-account` finds the account deleted, asks nothing,
//!     signs in to nothing, deletes the local state and says the account is deleted;
//!   - through a pipe it fails, saying `terminal`, and as an agent it fails, saying `skald
//!     delete-account`, `person's command` and `an agent cannot run`; either way it sends no request
//!     that deletes the account and starts no device authorization.
//! - On another device of a deleted account, `skald recv`, `skald send` and a running `skald
//!   helper` each say once, on one line holding `account` and `deleted`, that the server reports
//!   the account deleted, and name the machine's state directory; they send no envelope, history or
//!   revision. So does `skald link`, on a device that ran no helper when the account was deleted,
//!   once its session is refused; its `skald delete-account` then deletes its local state.
//! - A refused session alone deletes nothing: `skald delete-account` keeps the machine's state while
//!   the account lives. A run whose request to delete the account gets no answer, or one that is not
//!   the server's, says the deletion may have gone through.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod blocking;
mod confirming;
mod crashes;
mod deleting;
mod deletion;
mod offline;
mod other_devices;
mod refusals;
mod refused_sessions;

// The modules the tests of the channel history share (`history_common/mod.rs`), the admin rights'
// commands of the tests of admin rights, the terminal of the tests of the recovery key and its
// system key storage entry, and the new devices of the tests of linking with a link code.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../admin_rights/roles.rs"]
mod roles;
#[path = "../recovery_key/system.rs"]
mod system;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recovery_key/grants.rs"]
mod terminal_grants;
