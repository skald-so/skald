//! Handing messages over to agents: a recipient device hands each message to the agents it is for,
//! in the inbound mode `accept` (`docs/protocol/delivery.md`, "Hops" and "Device to agent").
//!
//! - A message goes to the agent in its content's `to` if it runs on this device; without `to`, to
//!   each agent of this device in the channel: each agent the device has run as in the channel's
//!   space. Content to a person goes to no agent; a receipt is never handed over.
//! - The device keeps each agent's messages in its local state until they are handed over, and
//!   marks a message handed over only after the handover: a crash in between repeats the handover
//!   and never loses it. Each handover carries the reference of its `msg_id`; the next message from
//!   a sending device after missing ones is marked as following them.
//! - A message waits for an agent until its envelope's `ttl` ends, at most the mailbox retention;
//!   then the device drops it for that agent.
//! - The person's `skald recv` shows each message once, whether or not an agent got it.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! whose agents run in `~/work/acme`, bound to `acme` by a rule; Bob's device is played by the
//! tests with an MLS state of its own and sends the content they write, with `to` or without.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages and of gap
//! detection:
//! - `skald --agent <name> recv`, in a folder bound to a space: runs as the agent `<name>`, which
//!   from then on is one of the device's agents in that space; shows each message handed over to
//!   it once, as one line with its text and the reference of its `msg_id`, and no ID, as the tests
//!   of compact output say (`compact_output/main.rs`), and marks it handed over once its output is
//!   written. The line of a message following missing ones is marked so.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod addresses;
mod agents;
mod held;
mod killed;
mod kinds;
mod missing;
mod recipients;
mod waiting;

pub use agents::{ACME_DIR, Agents, CLAUDE, CODEX, assert_not_handed, run_as_in};
pub use lines::assert_handed_line;

// The messages of the tests of gap detection, the kit of the tests of channel groups, the device
// the tests of receiving play over the delivery protocol, the device the tests of channel groups
// play, and the channel of the tests of channel messages.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// What an agent reads, as the tests of compact output have it.
#[path = "../compact_output/lines.rs"]
mod lines;

// The server's tests' readers of the KeyPackages API's answers and the person's browser; the
// server, the fixtures and the front of the tests of sending, the machines, the relay and people in
// spaces.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
