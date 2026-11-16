//! Sessions and signing out on the web: a web session lists the account's web and CLI sessions
//! and ends any of them; an ended session is refused at once, on every instance; a web session
//! expires 30 days after its sign-in. Against a `skald-server` on PostgreSQL with the GitHub
//! stand-in, and the OIDC stand-in where Google signs in; the web app in the browser on the harness
//! of `common/spa.rs`. Responses are checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, as these tests use it, each request stating the protocol version:
//! - `GET /account/sessions`, with a web session's cookie: `{"sessions": [<session>, ...]}`, every
//!   session of the account that has not ended, each as `{"session": <ID>, "kind": "web" |
//!   "device", "started_at": <date-time>, "last_used_at": <date-time>, "current": <bool>}`, and:
//!   for a web session, `"browser"` and `"os"`, read from the `User-Agent` of its sign-in, and
//!   `"expires_at"`, 30 days after its sign-in; for a device session, `"device_name"`, the name of
//!   the device it registered or connected as, and no `expires_at`. `current` is true on the
//!   request's own session only. An ID reveals nothing of the session's token. No IP address or
//!   location;
//! - `DELETE /account/sessions/{session}`: ends that session of the account, `204`;
//! - `POST /account/sessions/end-others`, no body: ends every session of the account but the
//!   request's own, `204`;
//! - `DELETE /session` with a web session's cookie: signs that web session out, `204`, as it does
//!   a device session with its bearer token;
//! - each of the three, with a web session, only from a page of the server, as its `Origin` says;
//!   a request from another site, or without `Origin`, ends nothing. A device session lists
//!   nothing and ends no other session. A refusal is a `4xx` with a problem details object;
//! - date-times are RFC 3339; `sessions.created_at` in the database is a session's sign-in.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../spaces/api.rs"]
mod api;
#[path = "../common/mod.rs"]
mod common;
#[path = "../web_sign_in/kit.rs"]
mod kit;

// The page helpers in `access.rs`, which the client's tests share, find these as siblings.
use common::{browser, spa};

mod access;
mod sessions;

mod ending;
mod expired_ids;
mod expiry;
mod instances;
mod last_use;
mod links;
mod list;
mod pages;
