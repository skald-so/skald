//! History entries of sent messages (`docs/protocol/membership.md`, "History entry", "Clean
//! epochs" and "Uploads").
//!
//! - `skald send` writes an entry of each message with an `a2a` body it sends in a channel, under
//!   the key of the epoch it encrypts the message in, if it judged that epoch clean: signed by the
//!   sending device, with the message's `msg_id` and content.
//! - The entry goes into the outbox with its envelope and is uploaded once the envelope is stored,
//!   in a `history` frame naming its `msg_id`, retried with the same bytes until the server takes or
//!   refuses it, also by a later run.
//! - A refused entry is not sent again, and the CLI says the message is not in the history. The
//!   copy is never written again under a later epoch's key.
//!
//! As the tests of the key chain (`key_chain/main.rs`): the CLI runs on Ada's machine, which creates
//! the spaces; Bob's and Carol's devices are played by the tests with an MLS state of their own. The
//! tests open the entries the server holds with the history keys Bob's device derives in each epoch
//! it is in, and check their signatures with the certificate of Ada's device. The front keeps each
//! `history` frame Ada's machine sends, and can lose or refuse the server's answers.
//!
//! The CLI, as these tests use it: `skald send <channel> <text>` and `skald recv`. A run that finds
//! a message of its device is not in the channel's history says so on a line naming the message's
//! `msg_id` and holding [`entries::NOT_IN_HISTORY`].
//!
//! The server's interface, as these tests use it: `GET
//! /spaces/{space}/channels/{channel}/history/entries`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod entries;
mod refused;
mod retries;
mod sent;
mod unclean;
mod withheld;

// The links of the tests of the key chain; the front of the tests of sending; the devices the tests
// of channel groups play; the talk of the tests of channel messages.
#[path = "../key_chain/chain.rs"]
mod chain;
#[path = "../sending/front.rs"]
mod front;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The server of the tests of signing in; the fixtures of the tests of sending; the kit of the tests
// of channel groups; the relay of the tests of signing in; people in spaces, of the tests of roster
// revisions.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The admin commands' kit of the tests of removing members; the joins of the tests of joining
// channels; the machines of the tests of signing in; the device the tests of receiving play.
#[path = "../member_removal/edits.rs"]
mod edits;
#[path = "../channel_joins/joins.rs"]
mod joins;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../receiving/peer.rs"]
mod peer;

// The server's tests' readers of the KeyPackages API's answers and the person's browser; the
// helpers of every client test; finding keys in bytes.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
