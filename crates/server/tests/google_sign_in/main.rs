//! Acceptance tests of Google sign-in on the server: `skald-server` against the local stand-in for
//! an OIDC provider, which plays Google, with the person's part in headless Chrome, so cookies act as
//! in a browser. No test calls a real Google endpoint. Responses are checked against the OpenAPI spec
//! in `api/openapi.json`.
//!
//! The server, as these tests use it: `common/google.rs`.

#[path = "../common/mod.rs"]
mod common;

mod accounts;
mod outputs;
mod refusals;
mod requests;
mod settings;
mod spec;
mod usernames;
mod web;

use common::accounts::Who;
use common::google::*;
use common::sign_in::{CALLBACK as GITHUB_CALLBACK, Jar};
