//! Link code runs of a new device without a session (`docs/protocol/membership.md`, "Linking a
//! device" and "Link code"): a new `client` device that has signed in nowhere opens a run, and the
//! server relays it, byte for byte and across instances, with the session of the account whose
//! person enters the pairing the run gives; nobody else, another account's sessions included, gets
//! or relays its messages. The run ends on a `link-abort` from either side, at the code's expiry,
//! once its device has a certificate and when a new run of its device replaces it. While its device
//! has no session the run is not given as waiting, so no linked device prompts for it; once its
//! device signs in to an account and registers there, it is, and the run goes on. Against a
//! `skald-server` with GitHub
//! sign-in through the stand-in, as in the tests of the link code relay; every HTTP response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, each request stating the protocol version; besides
//! those of the link code relay:
//! - `POST /links` of `{"device": <ID>, "hello": <bytes>}`, without a session: a `link-hello` of
//!   the new device `device`, which opens its run and replaces its open one; `201` with
//!   `{"pairing": <text>, "token": <text>}`. `pairing` is what the person enters on the linked
//!   device with the code, so the server pairs the run; the server sees it, never the code.
//!   `token` is the run's own: the new device's later requests carry it as `Authorization: Bearer
//!   <token>`, and it is no session.
//! - `GET /links/messages`, with the run's token: `{"messages": [<bytes>, ...]}`, the run's later
//!   messages from both sides, in the order the server took them.
//! - `POST /links/messages` of `{"message": <bytes>}`, with the run's token: a later message of the
//!   run, a `link-request` or a `link-abort`; taken with `201`.
//! - `POST /account/links/pairing` of `{"pairing": <text>}`, with a session: `200` with `{"hello":
//!   <bytes>}`, the hello of the run with that pairing, which is then the session's account's run;
//!   its later messages go by `session` through `POST /account/links/messages` and `GET
//!   /account/links/messages/{session}`. A pairing pairs one account's sessions.
//! - The certificate of the run's device ends it; the tests store it as the tests of sessions of a
//!   device linked by a code do: `POST /devices/{device}/challenge` and `POST
//!   /devices/{device}/session`.
//!
//! Bytes are base64url without padding (RFC 4648, §5); a message is its CBOR bytes. Refused, with a
//! problem details object: a `4xx`, `401` for a request whose token is no run's or session's. A
//! refused message is not stored or relayed.

#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests.
#[path = "../spaces/api.rs"]
mod api;

// The link code relay's messages and the requests of a run between two sessions.
#[path = "../link_relay/kit.rs"]
mod relay;

// Storing the certificate of a device a linked device issued.
#[path = "../linked_certificates/upload.rs"]
mod upload;

// A session of a device linked by a code, which stores its certificate.
#[path = "../device_key_sessions/kit.rs"]
mod by_key;

mod kit;

mod ends;
mod refusals;
mod runs;
mod waiting;
