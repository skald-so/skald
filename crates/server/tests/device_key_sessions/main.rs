//! Sessions of a device linked by a link code (`docs/protocol/membership.md`, "Linking a device"
//! and "Link code", step 4 of the new device): a new `client` device without a session, which a
//! linked device of its account certified through a run, stores its complete certificate and gets
//! a device session of that account, only as the holder of its own signing key. The server stores
//! the certificate with the checks and the devices limit of a certificate a session stores. The
//! session has no sign-in method; the server takes it as any device session and lists it with its
//! device among the account's sessions. It ends, its connections closed on every instance, when
//! its device is revoked, when it signs out and when a web session ends it. A device that signed
//! out and is not revoked gets a session again the same way, with no code. Against a
//! `skald-server` with GitHub sign-in through the stand-in, as in the tests of link runs without a
//! session, whose run each test's device first pairs with alice's account; every HTTP response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version, without a
//! session:
//! - `POST /devices/{device}/challenge`: `200` with `{"challenge": <bytes>}`, 32 random bytes.
//! - `POST /devices/{device}/session` of `{"challenge": <bytes>, "signature": <bytes>, "cert":
//!   <bytes>}`: `signature` by the device's signing key, with the label `skald device session`,
//!   over `["skald device session", challenge, origin]`, `origin` the origin of the server's public
//!   address as in `auth` (`docs/protocol/delivery.md`, "Connecting"); `cert` the device's complete
//!   certificate. `201`, or `200` for the same request again, a retry after a lost answer, with
//!   `{"token": <text>}`: a device session of the certificate's account, carried as
//!   `Authorization: Bearer <token>`. The account's `GET /devices` then gives the certificate as
//!   sent.
//! - Refused, with a problem details object, no session given and nothing stored: a `4xx`; `403`
//!   with the problem `https://skald.so/problems/devices-limit` over the devices limit.
//!
//! Bytes are base64url without padding (RFC 4648, §5); a certificate is its CBOR bytes. The
//! account's sessions, their ending and a device's connections are those of the tests of sessions
//! on the web; revocations those of the tests of revocations on the server.

#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests, the fixture and frames of the tests of storing
// sent envelopes, and the revocations of the tests of revocations on the server.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../mailbox/wire.rs"]
mod wire;

// The link code relay's messages, and the runs of a new device without a session.
#[path = "../link_relay/kit.rs"]
mod relay;
#[path = "../sessionless_links/kit.rs"]
mod runs;

// Storing the certificate of a device a linked device issued.
#[path = "../linked_certificates/upload.rs"]
mod upload;

// The account's sessions and a device's connections, of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod web;

mod kit;
mod linked;

mod ending;
mod issuing;
mod refusals;
