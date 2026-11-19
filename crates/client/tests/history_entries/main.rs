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
//! The CLI, as these tests use it: `skald send --all --channel <channel> <text>` and `skald recv`.
//! A run that finds a message of its device is not in the channel's history says so on a line
//! naming the message's `msg_id` and holding [`entries::NOT_IN_HISTORY`].
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

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
