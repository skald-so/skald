//! Clean epochs and the key chain in the CLI (`docs/protocol/membership.md`, "Clean epochs", "Key
//! chain" and "Uploads").
//!
//! - A member device judges each epoch of a channel once, when it enters it: by creating the
//!   group, a Welcome, a Commit, its own or another's, or a join or rejoin. An epoch is clean when
//!   every member device of it is admitted by the latest revision the device accepted. A revision
//!   it accepts later changes no judgment, either way.
//! - Once its Commit is stored, its sender uploads the link of the new epoch if it judged it clean,
//!   with `prev` the latest epoch before it that it judged clean; with none, as after a first join,
//!   or only epoch 0, there is no link.
//! - After its catch-up, a member device uploads the links it can make that the server lacks. It
//!   never uploads again a link the server has had, having taken it from the device or listed it
//!   to it, and never one of an epoch it judged unclean.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of
//! their own, so their runs make no uploads of their own. The tests open the links the server holds
//! with the history keys the played devices derive in each epoch they are in, the same as Ada's
//! device derives ("History key"). A link Bob's device uploads, it seals with its keys and sends in
//! a `history` frame on its connection. The front keeps each `history` frame Ada's machine sends,
//! and can close her connection at the next one carrying a link, which the server never gets. A
//! link that retention deletes is one whose row the test deletes from the server's database.
//!
//! The CLI, as these tests use it: the commands of the tests of channel messages, of joining
//! channels and of removing members.
//!
//! The server's interfaces, as these tests use them: `GET
//! /spaces/{space}/channels/{channel}/history/links`, and the table `history_links`, whose rows of
//! a channel a test deletes.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod chain;
mod joiners;
mod judged;
mod links;
mod retries;
mod seen;
mod unclean;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// What the tests of channel groups, channel messages, joining channels and removing members share.
#[path = "../member_removal/edits.rs"]
mod edits;
#[path = "../channel_joins/joins.rs"]
mod joins;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The front and fixtures of the tests of sending, and the device the tests of receiving play.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;

// The helpers of every client test, finding keys in bytes, and people in spaces.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../roster_revisions/world.rs"]
mod world;
