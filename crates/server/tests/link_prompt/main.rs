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

mod connections;
