//! A device's mailbox over HTTPS requests (`docs/protocol/delivery.md`, "Frames", "Connecting" and
//! "Mailbox"): a device that keeps no WebSocket sends, acks and long-polls its mailbox in requests
//! whose bodies are frames, each request proving its device as a WebSocket's `auth` does. The
//! server applies to them what it applies on a WebSocket: the protocol version, the refusals and
//! idempotency of sends, their order, the cursor and acks, receipts and expiry; instances wake a
//! waiting long-poll as they wake a WebSocket. Against a `skald-server` as in the tests of storing
//! sent envelopes, whose fixture and frames these tests share; every response is checked against
//! the OpenAPI spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, besides those of storing and delivering a mailbox:
//! - Each request is a `POST` stating the protocol version, with the device session's
//!   `Authorization: Bearer <token>` and `Content-Type: application/cbor`:
//!   - `/mailbox/send`, the body a `send` frame: `200`, `application/cbor`, its `stored` or
//!     `send-refused`;
//!   - `/mailbox/ack`, the body an `ack` frame: `204`;
//!   - `/mailbox/poll?wait=<seconds>`, the body a `sync` frame, whose cursor acks: `200`,
//!     `application/cbor`, a `batch` of the entries after the cursor in `seq` order and a
//!     `space-notice` for each roster revision stored since the device's previous long-poll answer.
//!     It answers at once when it has some or the wait is 0, otherwise with the first to come
//!     within the wait, empty if none does. A wait above the server's maximum, the setting
//!     `--long-poll-max-wait <SECONDS>`, ends at it.
//! - The proof: the header `Skald-Device-Proof`, the CBOR item `[challenge: bstr, device: id,
//!   signature: bstr .size 64]` in base64url without padding. `challenge` is from
//!   `POST /session/challenge` of the device's session and works once, used or refused;
//!   `signature` is by the device's signing key over `["skald request", challenge, origin]`,
//!   `origin` the server's public address. `docs/protocol/test-vectors.md` publishes its hex in
//!   the section "HTTPS proof".
//! - A request whose proof the server refuses reads, acks and stores nothing: `401` with a problem
//!   details object of the type `https://skald.so/problems/<reason>`, the reasons those of a
//!   WebSocket's `auth`: `auth-required` without a proof, `challenge-refused`, `device-refused`
//!   and `signature-refused`.
//! - A waiting long-poll ends with the refusal a new request would get when its session ends or
//!   its device is revoked, through any instance.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes, the entries of the tests of
// delivering a mailbox, and the revocations of the tests of revocations on the server; this binary
// uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../revocations/kit.rs"]
mod kit;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

#[allow(dead_code)]
mod https;

mod ends;
mod notices;
mod polls;
mod proofs;
mod sends;
