//! How the server tells the account's devices that one of its devices waits for linking
//! (`docs/protocol/delivery.md`; `docs/protocol/membership.md`, "Link code"): once a run of a link
//! code opens, each connection of the account's devices learns of it, and no connection of another
//! account does. Against a `skald-server` as in the tests of the link code relay, whose accounts and
//! messages these tests share, and the connections of the tests of storing sent envelopes.
//!
//! The interfaces, as these tests use them, besides those of the link code relay: a device's
//! WebSocket, authenticated as the device, whose first `sync` gets catch-up up to its end, an empty
//! `batch`. Once a run opens, the server sends each connection of the run's account a frame, a CBOR
//! item as "Frames" says; it sends a connection of another account none.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and their requests, the accounts and link messages of the tests of the
// link code relay, and the connections of the tests of storing sent envelopes; this binary uses
// some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../link_relay/kit.rs"]
mod kit;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

// The revisions of the spaces tests, the fixture of the tests of storing sent envelopes, the entries
// of the tests of delivering a mailbox, the requests of the tests of the mailbox over HTTPS and the
// streams of the tests of the mailbox over SSE; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox_https/https.rs"]
mod https;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;
#[allow(dead_code)]
#[path = "../mailbox_sse/sse.rs"]
mod sse;

mod connections;
mod transports;
