//! History key wraps to linked browsers (`docs/protocol/membership.md`, "Wraps", "Clean epochs",
//! "Key chain" and "Uploads").
//!
//! - Member devices wrap the key of each epoch they judged clean to each `browser` device with a
//!   valid certificate of an account among the channel's members, unless that member's `excluded`
//!   in the latest revision the device accepted lists it: sealed to the browser's `hpke` key and
//!   signed by the member device.
//! - The sender of each Commit uploads the new epoch's wraps. After its catch-up, a member device
//!   uploads the current epoch's wraps that the server lacks, so a browser linked later gets them
//!   and reads earlier epochs through the key chain.
//! - A revoked or excluded browser, a browser without a valid certificate and a browser of an
//!   account outside the channel get no wrap, and no epoch judged unclean is wrapped.
//!
//! As the tests of history entries (`history_entries/main.rs`): the CLI runs on Ada's machine,
//! which creates the spaces; Bob's and Carol's devices are played by the tests with an MLS state of
//! their own, so their runs make no uploads of their own. A browser is a `browser` device the test
//! writes to the server's database, its certificate issued by its account's device with the keys
//! the test holds, as Bob's device's or Ada's; the test opens its wraps with its HPKE key, checks
//! their signatures with the certificate of Ada's device, and compares the keys with those Bob's
//! device derives. The front keeps each `history` frame Ada's machine sends, and can close her
//! connection at the next one carrying a wrap of a channel, which the server never gets.
//!
//! The CLI, as these tests use it: `skald send --all --channel <channel> <text>`, `skald recv`, and
//! the revocations and revisions Ada's device signs that the tests store, as the tests of revoking
//! devices and of roster revisions do.
//!
//! The server's interfaces, as these tests use them: `GET
//! /spaces/{space}/channels/{channel}/history/wraps`, `.../history/links` and `.../history/entries`,
//! and the table `devices`, to which a test writes a browser.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod answered;
mod barred;
mod commands;
mod excluded;
mod hidden;
mod later;
mod lost;
mod refused;
mod unclean;
mod wrapped;
mod wraps;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the entries of
// the tests of history entries.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_entries/entries.rs"]
mod entries;
