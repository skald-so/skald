//! Space members and roles on the web: a session of a space's member gets the space's members as
//! the web app shows them, from its latest stored roster revision (`docs/protocol/membership.md`,
//! "Roster"): each member's handle and role, the GitHub login of its account's GitHub method while
//! it has one, the number of its account's linked devices and when the server stored the revision
//! that added it. No revision and no master key. A session of an account that is not a member gets
//! nothing of the space. Against a `skald-server` on PostgreSQL with the GitHub stand-in, its
//! accounts, revisions and revocations as in the tests of revocations on the server, and with the
//! OIDC stand-in, which plays Google, for an account without a GitHub method. Responses are checked
//! against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, each request stating the protocol version:
//! - `GET /spaces/{space}/members`, with a session of a member in the space's latest stored
//!   revision: `{"members": [<member>, ...]}`, each member of that revision once, as
//!   `{"handle": <handle>, "role": "admin" | "member", "github_login": <login>, "linked_devices":
//!   <count>, "joined_at": <date-time>}`: `github_login` only while the member's account has a
//!   GitHub method, absent otherwise; `linked_devices` counts the account's devices with a
//!   certificate that no counting revocation revokes, as `GET /account/devices` says `linked`;
//!   `joined_at` is when the server stored the revision that added the member. No revision, no
//!   master key, nor any field the spec names for one;
//! - another session, also of a member removed by a later revision, gets a `4xx` with a problem
//!   details object, `401` without a session or with an unknown one;
//! - date-times are RFC 3339.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The API calls, web sessions and date-times of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod sessions;

// The accounts, spaces, revisions and revocations of the tests of revocations on the server.
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

mod listed;

mod counts;
mod joined;
mod logins;
mod nothing;
mod revisions;
mod spec;
