//! Anchors objects in the CLI (`docs/protocol/membership.md`, "Anchors"): each `client` device signs
//! an anchors object whenever it accepts a roster revision, naming the latest revision it accepted
//! of each of its spaces, by the hash of that revision, and its pinned personal space. It writes it
//! before sending it and resends the latest until the server takes it, so a device whose anchors
//! never reached the server, such as one whose local state predates anchors objects, uploads them
//! at its next run. A revoked device signs none.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, which can lose the server's answers and answer a request itself, as a server
//! without anchors objects would, as in the tests of the personal space; a test plays the person on
//! the verification page in headless Chrome. Revisions the device did not sign are signed by the
//! tests with the keys of the account's devices and stored through the API, as in the tests of
//! roster revisions; a second device of the account is written to the server's database, as in the
//! tests of revoking a device.
//!
//! The interfaces, as these tests use them, beside the commands of the tests of spaces:
//! - `POST /account/anchors`, which stores an anchors object, and `GET /account/anchors`, which
//!   gives the last one of each device of the session's account, as the server's tests of anchors
//!   objects use them;
//! - `skald spaces` and `skald members <space>` accept the revisions the server holds of the spaces
//!   they use; `skald spaces`, as any run that connects, sends the anchors object the server has
//!   not taken.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod objects;
mod runs;
mod uploads;

// People in spaces, of the tests of roster revisions, and the devices the tests of sending write
// to the server's database, with the front those tests put before the relay.
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../roster_revisions/world.rs"]
mod world;

// The server, the relay and the machines of the tests of signing in.
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

// The helpers of every client test, finding keys in bytes, and the person's browser.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
