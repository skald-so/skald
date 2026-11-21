//! The bounds on requests without a session of link code runs and device key sessions
//! (`docs/protocol/membership.md`, "Link code" and "Device key session"): a run's token sends at
//! most 8 later messages of at most 4096 bytes each; an account enters at most 10 pairings the
//! server does not hold within 15 minutes; the server refuses a pairing once the run's device
//! registered on another account; a challenge for a device's session works once, for its device,
//! for less than a minute; and a device whose issuer is revoked gets no session. Against a
//! `skald-server` with GitHub sign-in through the stand-in, as in the tests of link runs without a
//! session; every HTTP response is checked against the OpenAPI spec in `api/openapi.json`.

#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

#[path = "../spaces/api.rs"]
mod api;

#[path = "../link_relay/kit.rs"]
mod relay;

#[path = "../linked_certificates/upload.rs"]
mod upload;

#[path = "../device_key_sessions/kit.rs"]
mod by_key;

#[path = "../sessionless_links/kit.rs"]
mod kit;

// The revocations of the tests of revocations on the server, with the fixture and frames of the
// tests of storing sent envelopes they use.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../mailbox/wire.rs"]
mod wire;

mod challenges;
mod revoked;
mod runs;

use crate::common::Response;
use crate::common::device_flow::{json, text};

/// Checks that `response` is the problem `kind` with `status`.
#[track_caller]
pub fn assert_problem(response: &Response, status: u16, kind: &str, what: &str) {
    assert_eq!(response.status, status, "{what}: {}", text(response));
    assert_eq!(
        json(response)["type"].as_str(),
        Some(format!("https://skald.so/problems/{kind}").as_str()),
        "{what}: {}",
        text(response)
    );
}
