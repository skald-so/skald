//! Joining a channel by itself and rejoining in the CLI (`docs/protocol/membership.md`, "Group
//! changes" and "Rejoining"; `docs/protocol/delivery.md`, "Mailbox" and "Gap detection").
//!
//! - A device admitted to a channel but outside its group joins by itself, with an external Commit
//!   from the latest GroupInfo, once its catch-up is done, when the GroupInfo does not list it and
//!   its mailbox held no Welcome for the channel; it checks the GroupInfo as a Welcome and uploads
//!   the new epoch's GroupInfo. So does a device whose Add another device should commit but has
//!   not, such as one the creator got no KeyPackage for.
//! - A device that refuses a GroupInfo stays outside the group, warns its user, and joins once the
//!   latest GroupInfo passes. A member device checks a joiner as an Add: it removes one that is not
//!   admitted, with a warning, and sends nothing in the channel until it is gone.
//! - A device that cannot process a channel's next handshake message, or that a Commit removed
//!   while it is admitted, rejoins with an external Commit that also removes its old leaf, warning
//!   its user in the second case. A Commit of an epoch it has passed is a replay, dropped.
//! - A device offline past the mailbox retention processes the Commits it missed in order, and gap
//!   detection reports the expired messages.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine;
//! Bob's and Carol's devices are played by the tests with an MLS state of their own; the tests sign
//! fixture revisions with the devices' keys. A Commit that never reaches Ada's device is one whose
//! mailbox entry the server's database drops before Ada's machine connects, as a proxy that keeps
//! it from the catch-up would. A device the server lets in that the members do not admit is one the
//! tests write to the server's database on Bob's account, its certificate chained to another master
//! key, as a server would set up a device of its own; it joins by an external Commit made on
//! mls-rs. A GroupInfo that lists a device that is not admitted is that of a fork of the channel's
//! group, signed by Bob's device, which the relay gives in place of the server's.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel groups, of channel
//! messages and of gap detection:
//! - a GroupInfo refused for a member or signer that is not admitted gives a warning line holding
//!   that device's ID, as a Welcome refused for its signer does;
//! - a joiner removed for not being admitted gives a warning line holding its device ID, as a
//!   device another device's Commit added does;
//! - a Commit that removed the device while it was admitted gives a warning line saying `removed`
//!   that holds the channel's ID.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod by_itself;
mod intruder;
mod joins;
mod offline;
mod rejoining;

// The front and fixtures of the tests of sending, and the device the tests of receiving play over
// the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The server's tests' readers of the KeyPackages API's answers, finding keys in bytes, and the
// helpers of every client test.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

// What the tests of channel groups, channel messages and gap detection share, and the mailbox
// retention and the times the server keeps.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;
#[path = "../../../server/tests/common/times.rs"]
mod times;

// The server, the relay and the machines of the tests of signing in, people in spaces, and the
// person's browser.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;
