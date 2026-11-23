//! Removing a deleted account's devices from channels (`docs/protocol/membership.md`, "Admitted
//! devices", "Wraps" and "Security considerations").
//!
//! - A device of an account the server reports deleted is not admitted: the first member device of
//!   each channel to learn of the deletion commits the Remove of its devices, which starts an epoch
//!   with a new history key, before it sends in the channel, and adds them nowhere. A device that
//!   processes such a Commit commits nothing more.
//! - The CLI learns of it on the `space-notice` and at each run that fetches the space's devices.
//! - Each member device tells its user once that the server reports the member deleted, and `skald
//!   members` marks it so.
//! - No link, wrap or entry of an epoch with the deleted account's devices, and no wrap to its
//!   recovery key or browsers, once the device has learned of the deletion.
//! - A report the server stops making lets the devices rejoin by themselves.
//! - A deleted admin counts for no only-admin warning.
//!
//! As the tests of revoking devices (`device_revocation/main.rs`): the CLI runs on Bob's machine,
//! which creates the space; the other people's devices are played by the tests with an MLS state
//! of their own. Ada has two devices, her first machine's and a second machine's, linked with a
//! link code as in the tests of link codes (`link_codes/main.rs`). Ada deletes her account through
//! the API with her first machine's session. A full copy of the disk of Ada's first machine is a
//! machine with a copy of every file under its root: in that test her first machine runs the CLI
//! too, and follows Bob's space from the anchors object her second device signs, as the tests of
//! spaces joined on another device do (`joined_spaces/main.rs`). A compromised server is played by
//! the front and the relay, which add an account to the deleted ones, hide GroupInfo and put
//! envelopes into a device's catch-up, and by the server's database, which serves the copy's
//! session and device again.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages, revoking
//! devices and admin rights:
//! - the run in which a device learns that the server reports a member account deleted shows one
//!   line that holds, as words, the member's handle and `deleted`; no later run of `skald recv` or
//!   `skald send` on the device shows such a line again;
//! - `skald members <space>`: the line of a member the server reports deleted, on standard output,
//!   holds `deleted`; no other member's line does;
//! - the only-admin warning of the tests of admin rights (`admin_rights/main.rs`).
//!
//! The server's interfaces, as these tests use them: `DELETE /account`; the `deleted` of `GET
//! /spaces/{space}/devices`, to which the relay adds an account; the tables `sessions`,
//! `sign_in_methods` and `devices`, which a test restores as a compromised server would.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod admins;
mod agent;
mod copy;
mod joining;
mod removal;
mod report;
mod scene;
mod uploads;

// The modules the tests of the channel history share (`history_common/mod.rs`), the entries of the
// tests of history entries, the runs through a helper of the tests of the helper, the new device of
// the tests of link codes, the role commands of the tests of admin rights, the terminal and the
// grants of the tests of the recovery key, the wraps of the tests of history wraps, the second
// machines of the tests of the history on new devices, the helpers of the tests of reading the
// history, and the revocations with `keep` of the tests of revoking linked devices.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../linked_revocation/linked.rs"]
mod linked;
#[path = "../link_codes/linking.rs"]
mod linking;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../admin_rights/roles.rs"]
mod roles;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recovery_key/grants.rs"]
mod terminal_grants;
#[path = "../history_wraps/wraps.rs"]
mod wraps;
