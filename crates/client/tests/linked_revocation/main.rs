//! Revoking linked devices in the CLI, and joining a channel whose group holds revoked devices
//! (`docs/protocol/membership.md`, "Revocation", "Group changes", "Admitted devices" and "Security
//! considerations").
//!
//! - The revocation command shows the devices the revoked device certified and lists in `keep`
//!   those its user names: they stay valid, at the account's devices and at other members', and
//!   stay in the channels, while the revoked device is removed. Without `keep`, they are revoked
//!   and removed too.
//! - A valid device outside a channel's group joins by itself from a GroupInfo whose signer and
//!   members that are not admitted are all revoked devices of member accounts, then commits their
//!   Remove before it sends anything: it sends nothing in its join epoch. Any other device that is
//!   not admitted, such as one the server joined or one excluded by the roster, still makes it
//!   refuse the GroupInfo and warn. A device linked later joins from its GroupInfo and reads its
//!   messages from the history.
//! - With the `stored` of that Remove lost, the next run removes them once, and the device sends
//!   nothing meanwhile.
//!
//! As the tests of the history on new devices (`history_devices/main.rs`): the CLI runs on Ada's
//! first machine, and the other machines of her account are linked by hand, each certified with
//! the keys of the machine that links it; they then join the account's channels by themselves.
//! Bob's device, where there is one, is played by the tests with an MLS state of its own, as in the
//! tests of channel messages (`channel_messages/main.rs`). A revocation the CLI has no command for
//! here, such as one a device of Ada's signed while the device that joins was not yet running, is
//! signed by the tests with that device's keys and stored from its session, as in the tests of
//! revoking devices (`device_revocation/main.rs`); so is a revision that bars a device. A device
//! the server joined is one the tests write to the server's database on Ada's account, its
//! certificate chained to another master key, listed in the GroupInfo of a fork of the channel's
//! group that a revoked device of Ada's signs, which the relay gives in place of the server's, as
//! in the tests of joining channels (`channel_joins/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald revoke-device <device> [--keep <kept>]...`: `--keep` names, by its ID, a device that
//!   `<device>` certified and that stays valid, once per device; the revocation's `keep` lists
//!   them. Without `--keep`, and with no terminal to ask its user on, it keeps none. Its output
//!   shows each device `<device>` certified on a line that is no warning, by its certificate's
//!   `name`;
//! - a GroupInfo refused for a member or signer that is not admitted gives a warning line holding
//!   that device's ID and the channel's, and `skald channels` lists the channel as one the device
//!   is not in.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod joining;
mod keeping;
mod linked;
mod refusing;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, and the helpers of the tests of reading the
// history.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;
