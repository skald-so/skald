//! The session's own account among a space's members on the web: `GET /spaces/{space}/members`
//! marks the member that is the request's session's account, and no other, so that the web app
//! tells whether that account is the space's only admin. Against a `skald-server` on PostgreSQL
//! with the GitHub stand-in, its accounts and revisions as in the tests of the space's members on
//! the web (`web_members/main.rs`). Responses are checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The API, as these tests use it, beside that of the tests of the space's members on the web:
//! - each member of `GET /spaces/{space}/members` may hold `"you"`, a boolean: `true` on the member
//!   whose account is the session's, for a web session and a device session alike; on every other
//!   member absent or `false`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The API calls, web sessions and date-times of the tests of sessions on the web.
#[path = "../web_sessions/sessions.rs"]
mod sessions;

// The accounts, spaces and revisions of the spaces tests, and the members' answer of the tests of
// the space's members on the web.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../web_members/listed.rs"]
mod listed;
#[path = "../spaces/rosters.rs"]
mod rosters;

mod marked;
