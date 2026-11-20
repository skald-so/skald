//! Which `service` devices a new roster revision may list (`docs/protocol/membership.md`,
//! "Roster"): the server refuses a revision that lists, in a service agent entry or in a channel's
//! `services`, a revoked `service` device or one on the account of no member of the revision, and
//! takes the same revision without it. Revisions it stored before stay as they are.
//!
//! Against a `skald-server` as in the tests of the service agents limit, whose space `acme` and
//! `service` devices, certified by their accounts' laptops and written to the database, these tests
//! share; revocations are stored as in the tests of revocations on the server. Every response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, beside those of the tests of roster revisions:
//! - `POST /spaces/{space}/revisions` of such a revision: a `4xx` with a problem details object,
//!   and nothing stored.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions, the fixture and frames of the tests of storing sent
// envelopes, the certified devices and revocations of the tests of revocations on the server, and
// the service agents of the tests of the service agents limit.
#[path = "../service_agents_limit/agents.rs"]
mod agents;
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

mod removed;
mod revoked;
