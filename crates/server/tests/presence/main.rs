//! Presence on the server (`docs/protocol/agents.md`, "Presence", "Uploads"):
//! - it takes a device's value in a channel only from a member device of the channel's current
//!   epoch, of kind `client` or `service`, with `device` itself and `epoch` the current one,
//!   each request proving its device as `/mailbox/send` does: a session alone uploads and reads
//!   nothing;
//! - it refuses others with `not-a-member` or `epoch-mismatch`, a `ct` over 65552 bytes with
//!   `too-large`, and bytes that do not decode, keeping nothing of them;
//! - it keeps one value per channel and device, the latest it took, and deletes it when a Commit it
//!   accepts removes the device from the group, or the device is revoked;
//! - it gives a channel's values, as the bytes it got, only to devices that may send in the channel
//!   (`docs/protocol/delivery.md`, "Send"): member devices, and a service token's device only with
//!   `send` for that channel.
//!
//! Against a `skald-server` with the scene of the tests of the server as the MLS Delivery Service:
//! its channel's group formed by test devices running MLS with `skald-crypto` (`uploads.rs`,
//! `refusals.rs`, `deletion.rs`, `access.rs`), or by alice's laptop with alice's `service` device,
//! the bot, of the tests of service requests (`services.rs`).
//!
//! The interfaces, as these tests use them:
//! - `PUT /spaces/{space}/channels/{channel}/presence`, the body `application/cbor`, a `presence`
//!   value's bytes: the proving device's value in the channel. Kept: `2xx`. Refused: `4xx` with a
//!   problem details object, of the type `https://skald.so/problems/<problem>` for `not-a-member`,
//!   `epoch-mismatch` and `too-large`.
//! - `GET /spaces/{space}/channels/{channel}/presence`: `200` with `{"values": [<bytes>]}`, the
//!   channel's values as the bytes the server got, in any order; refused, `4xx` with a problem
//!   details object.
//! - Each request states the protocol version and carries `Authorization: Bearer <token>`, a device
//!   session's or a service token's, and the device's proof in `Skald-Device-Proof`, as
//!   `/mailbox/send` takes them. Without the proof: `401` with the problem `auth-required`.
//!
//! Bytes in JSON are base64url without padding. Every response is checked against the OpenAPI spec.

// Each module uses a part of the shared ones.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The fixture and frames of the tests of storing sent envelopes; the spaces tests' accounts and
// revisions; the scene of the Delivery Service's tests; the requests of the mailbox over HTTPS and
// SSE and the connections of the tests of delivering a mailbox.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox/wire.rs"]
mod wire;

#[path = "../spaces/api.rs"]
mod api;
#[path = "../spaces/rosters.rs"]
mod rosters;

#[path = "../delivery_service/scene.rs"]
mod scene;

#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;

// The stage of the tests of service tokens, with the certificates of the tests of revocations, and
// the bot's tokens of the tests of service requests.
#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

mod kit;

mod access;
mod deletion;
mod refusals;
mod scopes;
mod services;
mod uploads;
