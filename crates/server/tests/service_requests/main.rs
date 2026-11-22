//! Requests of `service` devices on the server: a `service` device's token reaches only what its
//! permissions allow, and needs the device's key wherever a `client` device's session does
//! (`docs/protocol/delivery.md`, "Connecting" and "Send"; `docs/protocol/membership.md`, "MLS
//! profile", "KeyPackages" and "Uploads"). With the proof of its device's key, a live token
//! connects over WebSocket, long-poll and SSE: reads and acks need the receive permission, sends,
//! Commits included, the send permission and a channel of its grant. The token alone uploads
//! GroupInfo and its own device's KeyPackages, and reads the space's revisions, devices and
//! revocations, its grant, and the GroupInfo and history of its channels; with the proof, it also
//! reads its account's anchors objects. It is refused every other request. Its connections and
//! waiting requests end when it ends, on every instance.
//! Against a `skald-server` as in the tests of service tokens, whose stage these tests share: alice's
//! `service` device, the bot, certified by her laptop and listed as `ci/deploy-bot`, with tokens
//! that alice's session creates for `general`. Every response is checked against the OpenAPI spec
//! in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, besides those of service tokens and of the mailbox
//! over a WebSocket, HTTPS and SSE:
//! - `POST /session/challenge` with a live service token as `Authorization: Bearer <token>`: `200`,
//!   a challenge of the token, as for a device session; `401` with a problem details object for an
//!   ended token.
//! - A connection of the token: `/ws` whose `auth` names the token's device and signs one of its
//!   challenges, and `/mailbox/send`, `/mailbox/ack`, `/mailbox/poll` and `/mailbox/events` with the
//!   token as the bearer token and the device's proof on one of its challenges in
//!   `Skald-Device-Proof`, as a device session's requests make them.
//! - A request of the token that the server does not take: `401` with the problem `auth-required`
//!   without a proof, `device-refused` with the proof of a device other than the token's, and a
//!   problem details object for an ended token; a `4xx` with a problem details object for a
//!   long-poll, an ack or a stream without the receive permission, and for any request the token
//!   may not make. On a WebSocket without the receive permission, no entry comes and an ack
//!   deletes none. A send without the send permission is refused, as `send-refused` or a `4xx`
//!   with a problem details object; one in a channel outside the grant gets `send-refused` with
//!   `not-a-member`. A refused send stores nothing.
//! - The token alone: the requests of a device session with the token as its bearer token and no
//!   proof.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The stage of the tests of service tokens and the frames of the tests of storing sent envelopes;
// the requests of the mailbox over HTTPS and SSE and the connections of the tests of delivering a
// mailbox; the fixture of storing sent envelopes, the revocations of the tests of revocations and
// the MLS devices of the tests of the Delivery Service and the history uploads and reads of the
// tests of channel history; the spaces tests' accounts and revisions.
#[path = "../service_tokens/tokens.rs"]
mod tokens;
#[path = "../mailbox/wire.rs"]
mod wire;

#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;

#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../channel_history/kit.rs"]
mod history;
#[path = "../revocations/kit.rs"]
mod kit;
#[path = "../delivery_service/scene.rs"]
mod scene;

#[path = "../spaces/api.rs"]
mod api;
#[path = "../link_relay/kit.rs"]
mod link_kit;
#[path = "../spaces/rosters.rs"]
mod rosters;

// The anchors objects and requests of the tests of anchors objects.
#[path = "../anchors/objects.rs"]
mod anchors;

mod bot;

mod alone;
mod anchors_read;
mod connecting;
mod ends;
mod group;
mod permissions;
mod proofs;
mod scopes;
