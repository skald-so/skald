//! Copies of members' messages to service agents outside a channel's group
//! (`docs/protocol/delivery.md`, "Send"; `docs/protocol/README.md`, "Service grant"). A member's
//! device sends each such agent its own `hpke` copy; the server decides which agents get one:
//! - the space's members get the grants of its live tokens that have the receive permission, and
//!   no others;
//! - the server stores an `hpke` envelope to a `service` device only while a live token of it with
//!   the receive permission lists the envelope's channel; otherwise it refuses it with
//!   `unknown-recipient` and stores nothing of it.
//!
//! Against a `skald-server` as in the tests of service tokens, whose stage these tests share:
//! alice's `service` device, the bot, certified by her laptop and listed as `ci/deploy-bot`, with
//! tokens that alice's session creates. Every response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The interfaces, as these tests use them, besides those of service tokens and of storing sent
//! envelopes:
//! - `GET /spaces/{space}/service-grants` with a device session of a member of the space as
//!   `Authorization: Bearer <token>`: `200`, `{"grants": [<grant>, ...]}`, each the CBOR bytes of
//!   the grant of a live token of the space with the receive permission, in base64url without
//!   padding. A session of an account that is no member of the space, or no session, is refused
//!   with a `4xx` and a problem details object.
//! - A `send` of an `hpke` envelope from alice's laptop to the bot over the laptop's WebSocket:
//!   `stored`, or `send-refused` with the problem `unknown-recipient`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The spaces tests' accounts and revisions; the frames and fixture of the tests of storing sent
// envelopes, the entries of the tests of delivering a mailbox, the requests of the mailbox over
// HTTPS and SSE; the revocations and certified devices of the tests of revocations on the server;
// the stage of the tests of service tokens and the bot of the tests of its requests.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../common/mod.rs"]
mod common;
#[path = "../spaces/rosters.rs"]
mod rosters;

#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;
#[path = "../mailbox/wire.rs"]
mod wire;

#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

mod copies;

mod grants;
mod stored;
