//! Removing members and excluding devices in the CLI: only an admin's device removes a member from
//! a space or a channel, bars a member's device by its member's `excluded`, or adds a member of the
//! space to a channel, each by the next roster revision it signs; member devices then bring each
//! channel's group in line with the accepted revision (`docs/protocol/membership.md`, "Roster",
//! "Admitted devices" and "Group changes").
//!
//! - Each command signs the next revision as `skald create-channel` does: it first accepts the
//!   revisions the server has, applies its change to the latest, writes the revision before sending
//!   it and resends it until the server has taken it. On a device of a member that is no admin it
//!   refuses and sends nothing. The default channel, the first one, keeps every member.
//! - A member device that accepts a revision commits the Remove of the devices of a removed member,
//!   of a member removed from a channel and of an excluded device. The signing device commits the
//!   Add of a new channel member's devices when it is a member device of the channel; otherwise
//!   they join by themselves.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces, Ada an admin; Bob's and Carol's devices, members, are played by the
//! tests with an MLS state of their own; the tests sign fixture revisions with the devices' keys.
//! Bob's device, played, keeps its state as it was when it stops taking in its mailbox, as a full
//! copy of its disk taken then would; a compromised server, played by the test, hands it the
//! envelopes the front saw Ada's machine send, and the front puts what it sends into Ada's
//! catch-up. For a CLI on the device of a member that is no admin, Bob's machine creates the space
//! and Ada's device, made an admin, signs a revision that makes Bob a member.
//!
//! The CLI, as these tests use it, beside the commands of the tests of creating channels and of
//! channel messages, each on a space by its name and a member by its handle:
//! - `skald remove-member <space> <handle>` removes the member from the space and from every
//!   channel;
//! - `skald exclude-device <space> <device>`, the device's ID, adds the device to its member's
//!   `excluded`;
//! - `skald add-to-channel <space> <channel> <handle> --history yes|no`, the channel's ID, adds a
//!   member of the space to the channel with `history` as given;
//! - `skald remove-from-channel <space> <channel> <handle>` removes the member from the channel; for
//!   the default channel it fails, saying `default`, and sends nothing;
//! - each succeeds once the server has taken its revision; on a device of a member that is no
//!   admin it fails, saying `admin`, and sends nothing;
//! - a message the device drops gives a warning line holding the sending device's ID or the
//!   channel's ID, and never its text.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod adding;
mod copy;
mod edits;
mod lost;
mod refusals;

// The talk of the tests of channel messages, and the kit and played devices of the tests of
// channel groups.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// People in spaces, of the tests of roster revisions; the front and fixtures of the tests of
// sending; the device the tests of receiving play over the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../roster_revisions/world.rs"]
mod world;

// The server, the relay and the machines of the tests of signing in; the helpers of every client
// test; finding keys in bytes.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
