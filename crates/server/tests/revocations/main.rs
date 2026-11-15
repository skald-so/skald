//! Revocations on the server (`docs/protocol/membership.md`, "Revocation" and "Device checks"): the
//! server stores a revocation from a session of its account only if it counts against the account's
//! master key and the certificates the server holds, and gives each account's revocations with its
//! devices. A device is revoked when checks 3 and 4 fail with the stored revocations; checks 1 and 2
//! were made when the server took the certificate. At revocation, on every instance, the server
//! expires the device's mailbox entries ("Mailbox"), adding `expired` for those sent with `report`
//! true, closes its connections and adds it no later entry; wherever it checks a device's
//! certificate, a revoked device has none, and it does not count towards `--devices-limit`. Against
//! a `skald-server` as in the tests of storing sent envelopes, whose fixture and frames these tests
//! share; every HTTP response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, besides those of storing and delivering a mailbox:
//! - `POST /account/revocations` of `{"revocation": <bytes>}`, a revocation's CBOR bytes in
//!   base64url without padding, stating the protocol version, with the session's token: `201`
//!   once stored, `200` for the same bytes again. Refused with `400` and the problem
//!   `https://skald.so/problems/invalid-revocation`: bytes that do not decode as a `revocation`, or
//!   one that does not count for the session's account, such as one of another account. A
//!   malformed body: `400` with a problem; no session: `401`. A refused revocation is not stored.
//! - `GET /devices` and `GET /spaces/{space}/devices`: besides `devices`, `revocations`, an array
//!   of the stored revocations of the listed devices' accounts, each as the bytes the server got,
//!   once. The devices keep their certificates as the devices sent them.
//! - A revoked device's connections close, on every instance, within a second of the revocation's
//!   answer; a new one is refused with code 1008 and the reason `device-refused`. A send to it is
//!   refused with `unknown-recipient`; a roster revision it signs is refused.
//! - `devices`: one row per device, as the tests of the cloud limits write them; these tests add
//!   devices certified by another device of the account, and by another master key.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes, and the connections of the tests
// of delivering a mailbox; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

// The settings, problems and devices of the tests of the cloud limits; this binary uses some of them.
#[allow(dead_code)]
#[path = "../cloud_limits/limits.rs"]
mod limits;

mod kit;

mod checks;
mod connections;
mod mailboxes;
mod refusals;
mod storing;
