//! Channel messages over MLS in the CLI: a channel message is an MLS application message of the
//! channel's group (`docs/protocol/README.md`, "Envelope", "Scheme `mls`" and "Content";
//! `docs/protocol/delivery.md`, "Send" and "Mailbox"; `docs/protocol/membership.md`, "Admitted
//! devices").
//!
//! - `skald send` brings the channel's group in line first, then encrypts the content in the group
//!   and sends one `mls` envelope without `recipients`, with `report` true; the content's `counter`
//!   counts the device's messages in the channel. It saves the state and puts the envelope in the
//!   outbox before it sends, and retries with the same bytes.
//! - It sends nothing, and says why, while the group has a member that is not admitted, and when no
//!   other device is in the group: there is no plaintext path.
//! - `skald recv`, and the catch-up of `skald send`, decrypt each `mls` message in its channel's
//!   group in mailbox order and drop a replay. The sender may send in the channel when one of its
//!   account's `client` devices is in the group, under `hpke` too.
//!
//! As the tests of channel groups (`channel_groups/main.rs`): the CLI runs on Ada's machine, which
//! creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of their
//! own, reading the application messages they decrypt; the tests sign fixture revisions with the
//! devices' keys, and the front before the relay loses the server's answers and puts envelopes into
//! a device's catch-up, as a network or a compromised server would.
//!
//! The CLI, as these tests use it, beside the commands of the tests of sending, of receiving and of
//! channel groups:
//! - `skald send --all --channel <channel> <text>` with no other device in the channel's group
//!   fails, says `no device` can receive the message, and sends no message;
//! - while the group has a member that is not admitted, it fails, names that member's device and
//!   sends no message;
//! - `skald recv` shows a message as the tests of receiving say: its text once, the channel's ID,
//!   `from` and the sending device's certificate `name`; and for each `delivered` of the device's own
//!   message, a line holding the status, the message's ID and the recipient device's ID;
//! - a message it drops for failing a check is a warning line, holding the sending device's ID when
//!   the payload authenticates it, and never the message's text.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agent;
mod receiving;
mod sending;
mod talk;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The tests of channel groups' kit and the device they play.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;

// The server, the relay and the machines of the tests of signing in, and people in spaces.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The front and fixtures of the tests of sending, and the device the tests of receiving play over
// the delivery protocol.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
