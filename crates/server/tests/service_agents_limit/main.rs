//! The service agents a space may hold (`docs/dev/architecture.md`, "Cloud limits"): with the
//! setting, the server refuses a roster revision that adds service agent entries beyond it, and
//! takes one that keeps or lowers their number, so a space over a lowered limit still removes
//! agents and changes the rest. Unset, no revision is refused for its agents, as a self-hosted
//! server applies no cloud limits. The space's usage gives its service agents, those of its latest
//! revision, with the limit while the setting is there; the web app's Usage section shows them.
//! Against a `skald-server` as in the tests of storing sent envelopes, whose fixture these tests
//! share, with `service` devices certified by alice's laptop written to the database, as the tests
//! of revocations write them; the web app in the browser on the harness of `common/spa.rs`. Every
//! response is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, beside those of the tests of roster revisions:
//! - the setting `--service-agents-limit <N>`, also `SKALD_SERVICE_AGENTS_LIMIT`: the service agent
//!   entries a space's latest revision may list;
//! - `POST /spaces/{space}/revisions` of a revision that lists more service agent entries than the
//!   setting and more than the space's latest revision: `403` with the problem
//!   `https://skald.so/problems/service-agents-limit`, and nothing stored;
//! - `GET /spaces/{space}/usage`, with a session of a member: beside what the tests of usage on the
//!   web read, `"service_agents": {"count": <n>, "limit": <n>}`, the service agent entries of the
//!   space's latest revision and the setting's value, `limit` absent without the setting;
//! - the web app's Usage section, the page's main area, for the space the switcher chose: the
//!   service agents with their limit as "<count> of <limit>", with a bar, an element of role
//!   `progressbar` or `meter`, whose value is the count and maximum the limit.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, the fixture and frames of the tests of storing sent
// envelopes, and the certified devices of the tests of revocations on the server.
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

// The press of a button or link in the web app, of the tests of sessions on the web.
#[path = "../web_sessions/access.rs"]
mod access;
// `access.rs` finds these as siblings.
use common::{browser, spa};

mod agents;

mod limit;
mod lowered;
mod usage;
mod web;
