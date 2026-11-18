//! Service tokens on the server: an admin's device gets a scoped token for a `service` device of
//! its account that the space's latest revision lists in a service agent entry, with the device's
//! grant (`docs/protocol/README.md`, "Service grant"). The value is shown once and stored only as a
//! digest. A token ends at its expiry, the grant's, at its revocation, once its device is revoked
//! and once the latest revision lists its device in no service agent entry; the server then refuses
//! it and lists it no more. Against a `skald-server` as in the tests of storing sent envelopes,
//! whose fixture these tests share, with `service` devices certified by alice's or bob's laptop
//! written to the database, as the tests of revocations write them; every response is checked
//! against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version:
//! - `POST /spaces/{space}/service-tokens` of `{"device": <ID>, "grant": <bytes>, "permissions":
//!   [<permission>, ...]}`, with a device session's `Authorization: Bearer <token>` and the proof
//!   of one of its account's `client` devices in `Skald-Device-Proof`, as the mailbox requests
//!   over HTTPS make it. `device` is the `service` device, `grant` its grant's CBOR bytes, and the
//!   permissions, at least one, of `send`, `receive` and `queues`. `201`: the token's record and
//!   `"token": <value>`, the token itself, which no other answer holds. Refused, with a problem
//!   details object and nothing stored: `401` for a missing or refused proof; another `4xx` when
//!   the session's account is no admin in the latest revision, the device is not a `service`
//!   device of that account in a service agent entry of that revision, the grant fails its checks
//!   or no permission is given.
//! - A token's record: `{"id": <ID>, "device": <ID>, "owner": <name>, "agent": <name>,
//!   "permissions": [<permission>, ...], "channels": [<ID>, ...], "created_at": <date-time>,
//!   "last_used_at": <date-time> | null, "expires_at": <date-time>}`: its service agent's entry,
//!   the grant's channels and expiry, and its last use to the minute, `null` until its first
//!   request.
//! - `GET /spaces/{space}/service-tokens`, with a session of an account that is an admin in the
//!   space's latest revision, a device session's bearer token or a web session's cookie:
//!   `{"tokens": [<record>, ...]}`, the space's tokens that have not ended.
//! - `DELETE /spaces/{space}/service-tokens/{id}`, with such a session: `204`, the token ends. A
//!   web session's only from a page of the server, as its `Origin` says; from another site, or
//!   without `Origin`, it ends nothing.
//! - `GET /service-token` with the token as `Authorization: Bearer <value>`: `200`, the token's
//!   record and `"grant": <bytes>`, the grant as it was sent; `401` with a problem details object
//!   once the token has ended.
//!
//! Bytes are base64url without padding; date-times are RFC 3339.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, the fixture and frames of the tests of storing sent
// envelopes, the entries of the tests of delivering a mailbox, the revocations and certified
// devices of the tests of revocations on the server, and the proofs of the tests of the mailbox
// over HTTPS.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../mailbox/wire.rs"]
mod wire;

mod tokens;

mod creating;
mod ending;
mod listing;
mod refusals;
