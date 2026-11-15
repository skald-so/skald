//! KeyPackages and the sealed MLS state in the CLI: once the server takes its certificate, and on
//! each run that connects, a device keeps its KeyPackages on the server as
//! `docs/protocol/membership.md`, "KeyPackages", says: with fewer than 10 unused that expire in
//! more than 7 days, it uploads enough for 20; it replaces its last-resort one once it was handed
//! out or within 7 days of its expiry. It keeps its MLS state only sealed in the local state, with
//! the state key in key storage beside the device keys; each save overwrites the previous sealed
//! state and deletes it securely, so neither the database file nor its journal or WAL keeps it
//! (`architecture.md`, "Cryptography").
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay and, before it, the front of the tests of sending (`sending/front.rs`), at
//! the server's public address. A test plays the person on the verification page in headless
//! Chrome. Each machine keeps its state in a directory of its own and its secrets in a file there.
//! Ada's machine signs up and creates `acme`; the tests store a revision, signed with her device's
//! key, that adds Bob, whose session takes her device's KeyPackages through the API as an adding
//! device would.
//!
//! The interfaces, as these tests use them, beside those of the tests of roster revisions and of
//! receiving messages:
//! - `skald sign-in`, the account's first device, and `skald recv`, a run that connects, keep the
//!   device's KeyPackages;
//! - the server's KeyPackages API, as its tests use it (`crates/server/tests/key_packages`):
//!   `GET /devices/{device}/key-packages` gives `{"unused": <count>, "replace_last_resort":
//!   <bool>}`; `POST /key-packages` of `{"devices": [<ID>]}` hands out one KeyPackage, `200` with
//!   `{"key_packages": [{"device": <ID>, "key_package": <bytes>}]}`, none if the device has none;
//! - the server's database: the table `key_packages`, one row per KeyPackage, with the columns
//!   `device_id`, its device, `last_resort`, a `boolean`, and `expires_at`, a `timestamptz`: when
//!   the server takes it to expire. The tests move `expires_at` within 7 days.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod kit;
mod replenish;
mod sealed;

// The server's tests' readers of the KeyPackages API's answers, and the person's browser.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// People in spaces, and the front and fixtures of the tests of sending.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../roster_revisions/world.rs"]
mod world;

// Finding keys in bytes, and the helpers of every client test.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
