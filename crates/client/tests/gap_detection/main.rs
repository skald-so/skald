//! Gap detection in the CLI: the content's `counter` lets a recipient device notice a message it
//! did not get, and drop a repeated one (`docs/protocol/delivery.md`, "Gap detection" and
//! "Mailbox").
//!
//! - `skald recv` keeps, per channel, sending device and scheme, the highest `counter` it accepted
//!   and those it is missing, written with the entries and the cursor. The baseline: 0 under
//!   `hpke`; under `mls`, the first message from the sending device since this device last joined
//!   the channel.
//! - It accepts a counter above the highest, the counters between becoming missing, or a missing
//!   one; it drops any other as a duplicate or a replay.
//! - It warns its user of missing counters, naming the channel and the sending device, and marks
//!   the next message it shows from that device as following missing ones.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine;
//! Bob's device is played by the tests with an MLS state of its own. A message the server drops is
//! one Bob's device encrypts or seals and never sends: Ada's device sees what it would see.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages:
//! - a report of missing messages is a warning line saying `missing` that holds the channel's ID
//!   and the sending device's ID; one missing message gets one such line, once;
//! - the line that holds a shown message's ID and is no warning says `missing` when the message is
//!   the first the device shows from its sending device after missing ones, and not otherwise.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod counters;
mod gaps;
mod hpke;
mod joined;
mod killed;
mod missing;
mod rejoined;

// The tests of channel messages' channel, with the kit of the tests of channel groups and the
// device they play.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The front and fixtures of the tests of sending, and the device the tests of receiving play over
// the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// People in spaces, and the server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
