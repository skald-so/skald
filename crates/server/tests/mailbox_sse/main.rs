//! A device's mailbox over server-sent events (`docs/protocol/delivery.md`, "Frames", "Connecting"
//! and "Mailbox"): a device whose network passes SSE but not WebSocket gets its mailbox pushed on a
//! stream that proves its device as each HTTPS request of the mailbox does; it sends and acks over
//! those requests. Against a `skald-server` as in the tests of the mailbox over HTTPS, whose
//! fixture, requests and proofs these tests share; every response is checked against the OpenAPI
//! spec in `api/openapi.json`.
//!
//! The interfaces, as these tests use them, besides those of the mailbox over HTTPS:
//! - The stream: `GET /mailbox/events` stating the protocol version, with the device session's
//!   `Authorization: Bearer <token>`, `Accept: text/event-stream` and the proof header
//!   `Skald-Device-Proof`; the cursor in `Last-Event-ID`, 0 without it. `200` with
//!   `text/event-stream`, or a refusal as the requests' (`401`, a problem details object).
//! - Each event's `data` is one frame, its CBOR in base64url without padding (`README.md`,
//!   "Encoding"): a `message` or `status`, the event's `id` its `seq`, a `space-notice`, or the
//!   empty `batch` that ends catch-up. The stream acks its cursor, sends the entries after it in
//!   `seq` order, then the empty `batch` once it has sent what the mailbox held at opening, then
//!   each new entry and `space-notice`, through any instance.
//! - While idle, the stream sends a comment line, a keep-alive, at the interval of the server
//!   setting `--sse-keep-alive <SECONDS>`.
//! - The stream closes when its session ends or its device is revoked, through any instance.
//! - A stream gets no `space-notice` of a revision stored before it opened, also on an instance
//!   that did not listen to the others yet.
//! - An instance that does not listen to the others within 5 seconds answers a stream, and a
//!   long-poll with a wait, `500` with the problem `server-error`, and writes
//!   `skald-server: cannot answer a request: <why>` to its output; its error report, with
//!   `--sentry-dsn`, says why too.

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
// delivering a mailbox, the revocations of the tests of revocations on the server, and the requests
// and proofs of the tests of the mailbox over HTTPS; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox_https/https.rs"]
mod https;
#[allow(dead_code)]
#[path = "../revocations/kit.rs"]
mod kit;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

// The proxy of the tests of server instances, which refuses an instance's listening.
#[allow(dead_code)]
#[path = "../server_instances/proxy.rs"]
mod proxy;
// The stand-in for Sentry of the tests of error reporting.
#[allow(dead_code)]
#[path = "../error_reports/sentry.rs"]
mod sentry;

mod sse;

mod closes;
mod keep_alive;
mod listening;
mod refusals;
mod streams;
