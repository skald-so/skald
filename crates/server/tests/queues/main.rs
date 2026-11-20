//! A channel's queues and enqueued tasks on the server (`docs/protocol/queues.md`, "Queues",
//! "Tasks", "Task states", "Problems" and "Security considerations"):
//! - every channel has the queue `default`, with the default settings: `at-least-once`, a lease of
//!   300 seconds and 5 attempts; the server creates it when it issues the channel ID, a channel from
//!   before queues gets it from the migration, and nobody deletes it;
//! - a `client` device of a member of the channel creates a queue; one of the creator's account or
//!   of an admin of the space changes a setting other than `mode`, or deletes the queue with its
//!   tasks; others get `not-allowed`; a setting out of range and a name the channel has are
//!   refused;
//! - the server stores a task only from a device that may send in the channel (`docs/protocol/
//!   delivery.md`, "Send") and, for a `service` device, whose token allows the queue; once per
//!   `task` and per `idempotency_key` in its queue; sealed under the channel's current or previous
//!   epoch, with a `ct` within the message text limit; a stored task is `queued` with attempts 0;
//! - a repeated upload of a `task` from its device gets the first answer, whatever its bytes;
//! - it gives tasks only to devices that may send in the channel: member devices, and a service
//!   token's device with `send` for the channel;
//! - every queue request proves its device as `/mailbox/send` does, a `service` device's with its
//!   token: a session or a token alone makes none.
//!
//! Against a `skald-server` with the space of the tests of storing sent envelopes: alice, an admin,
//! and carol, a member who is no admin, both in the channel `general`, whose group alice formed with
//! bob's and carol's laptops; dave, a member of the space outside the channel; eve, outside the
//! space; and alice's `service` device, the bot, listed as a service agent outside the group, with
//! tokens alice's session creates.
//!
//! The interfaces, as these tests use them. Each request states the protocol version and carries
//! `Authorization: Bearer <token>`, a device session's or a service token's, and the device's proof
//! in `Skald-Device-Proof`, as `/mailbox/send` takes them; without the proof: `401` with the
//! problem `auth-required`. Refused: `4xx` with a problem details object, of the type
//! `https://skald.so/problems/<problem>` for the problems of `queues.md`.
//! - `GET /spaces/{space}/channels/{channel}/queues`: `200` with `{"queues": [<queue>]}`, a queue
//!   being `{"id": <queue ID>, "name": <name>, "mode": "at-least-once" | "at-most-once", "lease":
//!   <seconds>, "max_attempts": <n>}`.
//! - `POST /spaces/{space}/channels/{channel}/queues`, the body `application/json`, `{"name": ...}`
//!   with any of `mode`, `lease` and `max_attempts`, the others their defaults: `201` with the
//!   queue.
//! - `PATCH /spaces/{space}/channels/{channel}/queues/{queue}`, the body `application/json`, the
//!   settings to change: `200` with the queue.
//! - `DELETE /spaces/{space}/channels/{channel}/queues/{queue}`: `2xx`.
//! - `POST /spaces/{space}/channels/{channel}/queues/{queue}/tasks`, the body `application/cbor`,
//!   the `queued-task`'s bytes: `2xx` when stored. `duplicate` has the extension member `task`,
//!   the ID of the task already stored.
//! - `GET /spaces/{space}/channels/{channel}/queues/{queue}/tasks`: `200` with `{"tasks":
//!   [<task>]}`, in any order; `GET .../tasks/{task}`: `200` with the task. A task is
//!   `{"queued_task": <bytes>, "state": <state>, "attempts": <n>, ...}`, `queued_task` the bytes
//!   its device uploaded.
//!
//! Bytes in JSON are base64url without padding; IDs are in their text form. Every response is
//! checked against the OpenAPI spec.

// Each module uses a part of the shared ones.
#![allow(dead_code)]

#[path = "../common/mod.rs"]
mod common;

// The stage of the tests of service tokens, with the certificates of the tests of revocations and
// the bot's tokens of the tests of service requests; the scene of the Delivery Service's tests;
// the spaces tests' accounts and revisions; the answers the tests of presence check; the fixture
// and frames of the tests of storing sent envelopes; the requests of the mailbox over HTTPS and SSE
// and the connections of the tests of delivering a mailbox.
#[path = "../spaces/api.rs"]
mod api;
#[path = "../service_requests/bot.rs"]
mod bot;
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[path = "../mailbox_https/https.rs"]
mod https;
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[path = "../presence/kit.rs"]
mod presence;
#[path = "../revocations/kit.rs"]
mod revocations;
#[path = "../spaces/rosters.rs"]
mod rosters;
#[path = "../delivery_service/scene.rs"]
mod scene;
#[path = "../mailbox_sse/sse.rs"]
mod sse;
#[path = "../service_tokens/tokens.rs"]
mod tokens;
#[path = "../mailbox/wire.rs"]
mod wire;

mod kit;

mod access;
mod defaults;
mod enqueue;
mod settings;
