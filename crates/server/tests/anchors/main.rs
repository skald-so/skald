//! Anchors objects on the server (`docs/protocol/membership.md`, "Anchors"): the server takes from a
//! session of an account an anchors object of that account whose signature verifies with the
//! certificate of its `device`, a `client` device of the account with a valid certificate that is
//! not revoked. It keeps the last one per device, also once the device is revoked; the same bytes
//! again count as stored. It gives them, as the bytes it got, only to the account's sessions.
//! Against a `skald-server` as in the tests of revocations, whose fixture and kit these tests share;
//! every HTTP response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, beside those of revocations:
//! - `POST /account/anchors` of `{"anchors": <bytes>}`, an anchors object's CBOR bytes in base64url
//!   without padding, stating the protocol version, with the session's token: `201` once stored,
//!   replacing the last one of its device; `200` for the bytes the server holds for its device.
//!   Refused with `400` and the problem `https://skald.so/problems/invalid-anchors`: bytes that do
//!   not decode as an `anchors`, an object of another account than the session's, or one whose
//!   `device` is not a `client` device of the account with a valid certificate that is not revoked,
//!   or whose signature does not verify with that certificate's key over `["skald anchors", body]`.
//!   A malformed body: `400` with a problem; no session: `401`. A refused object is not stored.
//! - `GET /account/anchors`, with the session's token: `200` with `{"anchors": [<bytes>, ...]}`,
//!   the last anchors object of each device of the session's account, each as the bytes the server
//!   got, in base64url without padding; no session: `401`.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes, and the kit of the tests of
// revocations; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../revocations/kit.rs"]
mod kit;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod objects;

mod refusals;
mod storing;
