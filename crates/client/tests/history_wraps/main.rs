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
//! The CLI, as these tests use it: `skald send <channel> <text>`, `skald recv`, and the revocations
//! and revisions Ada's device signs that the tests store, as the tests of revoking devices and of
//! roster revisions do.
//!
//! The server's interfaces, as these tests use them: `GET
//! /spaces/{space}/channels/{channel}/history/wraps`, `.../history/links` and `.../history/entries`,
//! and the table `devices`, to which a test writes a browser.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod answered;
mod barred;
mod excluded;
mod hidden;
mod later;
mod lost;
mod unclean;
mod wrapped;
mod wraps;

// The modules of other test targets, by name: the server's tests' readers of the KeyPackages API's
// answers and the person's browser; the links of the tests of the key chain; the server, relay and
// machines of the tests of signing in; the helpers of every client test; the admin commands' kit of
// the tests of removing members; the entries of the tests of history entries; the fixtures and
// front of the tests of sending; the joins of the tests of joining channels; finding keys in bytes;
// the kit and played devices of the tests of channel groups; the device the tests of receiving
// play; the revocations of the tests of revoking devices; the talk of the tests of channel
// messages; people in spaces, of the tests of roster revisions.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../key_chain/chain.rs"]
mod chain;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../member_removal/edits.rs"]
mod edits;
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../channel_joins/joins.rs"]
mod joins;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../device_revocation/revocations.rs"]
mod revocations;
#[path = "../channel_messages/talk.rs"]
mod talk;
#[path = "../roster_revisions/world.rs"]
mod world;
