//! Devices on the web: a session of an account gets the account's devices as the web app shows
//! them, each with its name, kind, when it registered and whether it is linked, waiting for linking
//! or revoked by the server's checks (`docs/protocol/membership.md`, "Revocation" and "Device
//! checks"), and no key or certificate. Against a `skald-server` on PostgreSQL with the GitHub
//! stand-in, its accounts and revocations as in the tests of revocations on the server. Responses
//! are checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, each request stating the protocol version:
//! - `GET /account/devices`, with a web session's cookie: `{"devices": [<device>, ...]}`, every
//!   device of the session's account, each as `{"device": <ID>, "name": <name>, "kind": "client" |
//!   "browser" | "service", "registered_at": <date-time>, "state": "linked" | "waiting" |
//!   "revoked"}`: `linked` with a certificate whose device no counting revocation revokes,
//!   `waiting` without a certificate, `revoked` once one does. No key and no certificate, nor any
//!   field the spec names for one. Without a session, or with an unknown one: `401` with a problem
//!   details object;
//! - date-times are RFC 3339.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The accounts, revisions and revocations of the tests of revocations on the server.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../mailbox/wire.rs"]
mod wire;

// The API calls, web sessions and date-times of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod sessions;

mod shown;

mod sessions_only;
mod spec;
mod states;
