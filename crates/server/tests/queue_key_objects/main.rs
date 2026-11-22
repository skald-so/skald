//! A channel's queue key objects on the server (`docs/protocol/queues.md`, "Queue key object";
//! `docs/protocol/membership.md`, "Uploads"):
//! - it keeps the first object per channel and epoch, only from a member device of that epoch,
//!   each on its connection, which proves the device's key: a session or a service token alone
//!   uploads none; a later object of the epoch changes nothing, and the same bytes again count as
//!   kept;
//! - it refuses, keeping nothing, an object from a device outside its epoch or a revoked one, from a
//!   `service` device outside the group, for an epoch the channel has not had, naming another
//!   channel than its upload, or bytes that do not decode;
//! - it gives an epoch's object, as the bytes it got, only to devices that may send in the channel
//!   (`docs/protocol/delivery.md`, "Send"): member devices, and a service token's device with
//!   `send` for the channel, each proving its key as the queue requests do.
//!
//! Against a `skald-server` with the world of the tests of queues (`queues/main.rs`): alice, an
//! admin, and carol, a member who is no admin, both in the channel `general`, whose group alice
//! formed with bob's and carol's laptops at epoch 1; dave, a member of the space outside the
//! channel; eve, outside the space; and alice's `service` device, the bot, listed as a service
//! agent outside the group, with tokens alice's session creates.
//!
//! The interfaces, as these tests use them, besides those of the tests of queues and of channel
//! history (`channel_history/main.rs`):
//! - On a device's connection, client → server, a `history` frame whose item is `[4, queue-key:
//!   bstr .cbor queue-key]`. Server → client, its answer: `history-stored`, kept or one kept in its
//!   place before, or `history-refused` with a problem, keeping nothing: `not-a-member` for a
//!   device that is no member of the object's epoch, or a revoked one; `epoch-mismatch` for an
//!   epoch the channel has not had; `malformed` for bytes that do not decode as a `queue-key` or an
//!   object naming another channel than its upload.
//! - `GET /spaces/{space}/channels/{channel}/queue-keys/{epoch}`, with the bearer token and the
//!   device's proof as the queue requests take them: `200` with `{"queue_key": <bytes>}`, the object
//!   of the epoch the server keeps, as the bytes it got; `404` with a problem details object when
//!   it keeps none; `401` with `auth-required` without the proof; refused, a `4xx` with a problem
//!   details object, for a device that may not send in the channel.
//!
//! Bytes in JSON are base64url without padding. Every response is checked against the OpenAPI spec.

// Each module uses a part of the shared ones.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The world of the tests of queues: the fixture's people, the channel's group and the queue
// requests.
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../queues/kit.rs"]
mod kit;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../delivery_service/scene.rs"]
mod scene;

// Sessions, proved HTTPS requests and their problems.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../presence/kit.rs"]
mod presence;

// The bot, its tokens and the modules they need.
#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../mailbox_sse/sse.rs"]
mod sse;
#[path = "../service_tokens/tokens.rs"]
mod tokens;

// Devices' connections and revocations.
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../mailbox/wire.rs"]
mod wire;

// The history uploads of the tests of channel history.
#[path = "../channel_history/kit.rs"]
mod history;

mod objects;

mod keeping;
mod reading;
mod refusals;
