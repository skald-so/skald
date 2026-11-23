//! A member's messages in the CLI once the server has erased its deleted account: 30 days after the
//! account deleted itself, the server keeps, for each space it shared, its revisions, the history
//! entries its devices wrote and those devices' certificates, so the other members' devices still
//! show its earlier messages, a device linked after the erasure too.
//!
//! As the tests of reading the history (`history_reading/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's device is played by the tests with an MLS state of its own, and
//! says each message as its own client would, uploading its entry. Ada's new machine is linked with
//! a link code, as in the tests of linking with a link code (`link_codes/main.rs`).
//!
//! The CLI, as these tests use it: `skald history`, `skald recv` and `skald send`, as the tests of
//! reading the history use them, and `skald sign-in` and `skald link`, as the tests of linking with
//! a link code use them.
//!
//! The server, as these tests use it:
//! - `DELETE /account` with a device session's `Authorization: Bearer <token>` deletes its account,
//!   `204`, as the server's tests of account deletion use it;
//! - `accounts.deleted_at` in the database is when the account deleted itself: the tests move it
//!   back 30 days;
//! - the server then erases the account within 30 seconds: no row of its database holds the
//!   account's master public key but those holding a revision of a space it shared, each read as
//!   one `<row>` of the database's XML.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod erased;

// The modules the tests of the channel history share (`history_common/mod.rs`), the helpers of the
// tests of the channel history on new devices and of reading the history, and the new device and
// terminal of the tests of linking with a link code.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recovery_key/grants.rs"]
mod terminal_grants;
