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

mod joiners;
mod judged;
mod links;
mod retries;
mod seen;
mod unclean;

// The modules the tests of the channel history share (`history_common/mod.rs`), this target's
// `chain` among them.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
