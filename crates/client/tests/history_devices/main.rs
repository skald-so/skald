//! The channel history on a member's new device and on a revoked device's disk
//! (`docs/protocol/membership.md`, "Key chain", "Access" and "Security considerations").
//!
//! - A member's new device, linked after messages were sent, reads the channel's history once a
//!   member device has run since its join: those messages too, from before a rotation of the
//!   history key and from before the device existed, back along the key chain.
//! - A new device of an account whose `history` is false reads from the epoch that added the
//!   account to the channel, not only from its own first epoch, and nothing before.
//! - A full copy of a revoked device's disk reads none of the history written after the device's
//!   revocation, even from a compromised server that hides the revocation and hands it every entry
//!   and link: no history key it holds opens a later entry or link.
//!
//! As the tests of reading the history (`history_reading/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of
//! their own, and say each message as their own client would, uploading its entry, and the sender
//! of a played Commit uploads the new epoch's link. A full copy of the disk of Ada's machine is a
//! machine with a copy of every file under its root, as in the tests of revoking devices.
//!
//! Until devices are linked by a link code, a test links a second machine of Ada's account by hand,
//! as the link will: the machine signs in as no first device, so its device is registered and waits
//! for linking; the test certifies that device with the keys of Ada's first machine and sets the
//! certificate up in the server's database; then it writes the certificate, the pinned master key
//! and each space as the first machine accepted it, its latest revision, into the second machine's
//! local state. The second machine then joins the account's channels by itself.
//!
//! The CLI, as these tests use it: `skald sign-in --first-device no`, `skald recv`, `skald send`,
//! `skald revoke-device` and `skald history`, as the tests of signing in, channel messages,
//! revoking devices and reading the history use them; and `SKALD_CLOCK_OFFSET`, as the tests of
//! scheduled rotation use it.
//!
//! The local state, as these tests write it to link a machine by hand: in `state.sqlite3`, the
//! table `device`, whose `link` becomes `certified` with the pinned `master` and the `cert` in
//! CBOR, and the table `space`, whose `accepted` is the latest revision, as the only one the
//! machine starts from. The server's table `devices`, whose `cert` a test sets.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod devices;
mod new_device;
mod revoked;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the helpers of
// the tests of scheduled rotation and reading the history, and the server's tests' times.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_rotation/aged.rs"]
mod aged;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../../../server/tests/common/times.rs"]
mod times;
