//! Signing in to the web app: a person signs up or signs in with GitHub or Google in one click and
//! lands in the cabinet under their account. Against a `skald-server` on PostgreSQL with the GitHub
//! stand-in and the OIDC stand-in, which plays Google; in the browser on the harness of
//! `common/spa.rs`. Responses are checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, beside `common/google.rs` and `spaces/main.rs`:
//! - `GET /auth/providers`, without a session: `{"providers": ["github" | "google", ...],
//!   "sign_up": "open" | "closed"}`, each provider the server has settings for and whether its
//!   sign-up is open, and nothing of the lists of who may sign up;
//! - `GET /session` of a web session names the provider it signed in with:
//!   `"sign_in_method": "github" | "google"`;
//! - `GET /spaces`: the account's personal space has `"personal": true`; any other space has
//!   `"personal": false` or no `personal`;
//! - the migration that records the sign-in method of each web session ends the web sessions from
//!   before it and keeps the device sessions.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../spaces/api.rs"]
mod api;
#[path = "../common/mod.rs"]
mod common;
#[path = "../spaces/rosters.rs"]
mod rosters;

mod kit;
mod methods;
mod migration;
mod pages;
mod providers;
mod spaces;
