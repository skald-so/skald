//! Joining a channel's group by itself and rejoining it: `docs/protocol/membership.md`, "Group
//! changes" and "Rejoining", and `mls-prototypes.md`, "For the crypto core". A device joins by an
//! external Commit from the channel's latest GroupInfo; the caller checks the GroupInfo's signer
//! and members before the group is saved, and a refused GroupInfo leaves no group. A rejoin's
//! external Commit also removes the device's old leaf. A member that processes another device's
//! external Commit gets its joiner as the device added and the leaf it removes, and refuses one
//! that removes another device's leaf. Every step loads the state the previous step sealed.
//! Natively and in WASM.
//!
//! The interface, as these tests use it, beside the steps of the tests of MLS on a device:
//! - `MlsDevice::join_by_itself(&mut self, channel: ChannelId, group_info: &[u8], now: u64, check:
//!   impl FnOnce(&Joining) -> bool) -> Result<Vec<u8>, MlsError>`: joins `channel`'s group at `now`
//!   by an external Commit from `group_info`, a GroupInfo in an `MLSMessage` with the
//!   `ratchet_tree` and `external_pub` extensions; gives the Commit, a PublicMessage. `check` gets
//!   the GroupInfo's signer and every member it lists before the group is saved; `false` refuses it
//!   with `MlsError::Refused`. A GroupInfo refused, by `check` or for its tree, signature or
//!   channel, leaves no group. If the GroupInfo lists a leaf with the device's own signature key,
//!   the Commit also removes that leaf. mls-rs applies an external Commit at once, so the device is
//!   in the new epoch when the step ends, its history key kept, with no Commit pending.
//! - `MlsDevice::process_commit` of another device's external Commit: `GroupChange::sender` and the
//!   one device of `GroupChange::added` are the joiner, and `GroupChange::removed` holds the leaf
//!   the Commit removes, if any.

// The acceptance tests of joining by itself, which fail until the code they test lands; the
// implementation removes this gate and the `acceptance` feature.
#![cfg(feature = "acceptance")]

#[path = "../common/devices.rs"]
mod devices;
#[path = "../mls/fixtures.rs"]
#[allow(dead_code)]
mod fixtures;
#[path = "../mls/foreign.rs"]
#[allow(dead_code)]
mod foreign;
#[path = "../mls/group_info.rs"]
#[allow(dead_code)]
mod group_info;
#[path = "../mls/steps.rs"]
#[allow(dead_code)]
mod steps;
#[path = "../mls/wire.rs"]
#[allow(dead_code)]
mod wire;

mod hostile;
mod joins;
mod rejoins;
