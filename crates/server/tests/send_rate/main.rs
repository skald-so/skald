//! The send rate limit (`docs/dev/architecture.md`, "Cloud limits" and "Security";
//! `docs/protocol/delivery.md`, "Send"): a server setting gives the messages one device may have
//! stored per minute, so that no loop or flood of one device goes past it. Unset, no limit
//! applies, as a self-hosted server applies no cloud limits. Against a `skald-server` as in the
//! tests of storing sent envelopes, whose fixture and frames these tests share; every HTTP response
//! is checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The settings, each also an environment variable:
//! - `--send-rate-limit <N>`, `SKALD_SEND_RATE_LIMIT`: the messages one device may have stored
//!   within a window. A message is a sending device's `msg_id`, counted as the monthly limit counts
//!   it: once, whatever its envelopes; a retry does not count, and neither does a refused message
//!   or a handshake message, a Commit or a Welcome (`docs/protocol/delivery.md`, "Mailbox").
//!   A message of a device that has had `N` messages stored within the window that ends at the
//!   send is refused with `send-refused` and the problem `rate-limited`, and nothing of it is
//!   stored; a handshake message is never refused for it. Each device has its count, across every
//!   instance of the server on one database.
//! - `--send-rate-window <SECONDS>`, `SKALD_SEND_RATE_WINDOW`: the window, in seconds; unset, a
//!   minute. The tests shorten it; the server may leave it out of its `--help`.

// This binary uses some of each of the modules below.
#[path = "../common/mod.rs"]
mod common;

// The frames and fixture of the tests of storing sent envelopes.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

// The spaces tests' accounts and revisions, which the fixture builds on.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The sends of the tests of the cloud limits, the connections of the tests of delivering a mailbox
// and the requests of the tests of a mailbox over HTTPS.
#[allow(dead_code)]
#[path = "../mailbox_https/https.rs"]
mod https;
#[allow(dead_code)]
#[path = "../cloud_limits/limits.rs"]
mod limits;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox_delivery/listener.rs"]
mod listener;

// The scene of the tests of the server as a channel's MLS Delivery Service.
#[allow(dead_code)]
#[path = "../delivery_service/scene.rs"]
mod scene;

mod rate;

mod counting;
mod devices;
mod handshakes;
mod settings;
mod window;
