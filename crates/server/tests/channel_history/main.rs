//! A channel's history on the server (`docs/protocol/membership.md`, "History", "Uploads" and
//! "History entry"):
//! - it takes a link, a wrap or an entry only from the device a connection proves (`docs/protocol/
//!   delivery.md`, "Connecting"): a session alone uploads nothing;
//! - it keeps the first link per channel, epoch and `prev` and the first wrap per channel, epoch and
//!   recipient, each only from a member device of its epoch; a later one changes nothing, and the
//!   same bytes again count as stored;
//! - it keeps one entry per sending device and `msg_id`, which the upload names, only for the
//!   channel's current or previous epoch and only from a member device of the current epoch;
//! - it refuses an upload that does not decode, or names another channel or an epoch the channel
//!   has not had;
//! - it gives them, as the bytes it got, only to sessions of accounts the latest revision lists as
//!   the channel's members;
//! - to an account whose `history` is false in the latest revision, only the links whose `prev` is
//!   at or after the epoch whose Commit added the account, and the wraps and entries of that epoch
//!   and later ones ("Access");
//! - a database migrated from before history keeps the member devices of each followed group's
//!   current epoch;
//! - with the history retention set, it deletes entries past it and the links and wraps of epochs
//!   before theirs, and refuses those later; without it, it keeps everything (`retention.rs`).
//!
//! Against a `skald-server` with the scene of the tests of the server as the MLS Delivery Service:
//! its channel's group, formed by test devices running MLS with `skald-crypto`, the member epochs
//! the server follows, and the fixture revisions of the tests of storing sent envelopes, with every
//! channel member reading the whole history.
//!
//! The interfaces, as these tests use them, besides those of the Delivery Service's tests:
//! - On a device's connection, client → server, `history = [11, upload: uint, space: id, channel:
//!   id, item]`, `item` one of `[1, link: bstr .cbor history-link]`, `[2, wrap: bstr .cbor
//!   history-wrap]` and `[3, msg_id: id, entry: bstr .cbor history-entry]`; `upload` is the
//!   client's number for the upload, which the answer names.
//! - Server → client, the answer: `history-stored = [12, upload: uint]`, kept or kept before, or
//!   `history-refused = [13, upload: uint, problem: tstr]`, with nothing stored.
//! - `GET /spaces/{space}/channels/{channel}/history/links`, `.../history/wraps` and
//!   `.../history/entries`: `200` with `{"links": [<bytes>]}`, `{"wraps": [<bytes>]}` and
//!   `{"entries": [<bytes>]}`, each item as the bytes the server got, in any order; refused, `4xx`
//!   with a problem details object; no session, `401`.
//!
//! Bytes in JSON are base64url without padding. Every response is checked against the OpenAPI spec.

// The fixture and frames of the tests of storing sent envelopes; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The scene of the Delivery Service's tests; this binary uses some of it.
#[allow(dead_code)]
#[path = "../delivery_service/scene.rs"]
mod scene;

#[path = "../common/mod.rs"]
mod common;

mod kit;

mod access;
mod earlier_history;
mod first_kept;
mod migration;
mod refusals;
mod retention;
mod storing;
