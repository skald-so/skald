//! Delivering a mailbox (`docs/protocol/delivery.md`, "Mailbox" and "Receipts"): on a connection
//! authenticated as a device, the server sends no entry before the connection's first `sync`; the
//! first `sync` acks and starts catch-up after its cursor, which ends with an empty `batch`; then
//! each new entry is pushed as it is added, to each of the device's connections in `seq` order. A
//! later `sync` and each `ack` only ack: the server deletes every entry up to the cursor, and an
//! envelope once no entry refers to it. An ack that covers a `message` sent with `report` true adds
//! `delivered` to the sending device's mailbox. Against a `skald-server` as in the tests of storing
//! sent envelopes, whose fixture and frames these tests share.
//!
//! The interfaces, as these tests use them, besides those of storing sent envelopes:
//! - The frames of "Frames": `sync = [4, cursor: uint]` and `ack = [7, seq]` from the device;
//!   `message = [5, seq, envelope]`, `status = [6, seq, delivery-status]` and
//!   `batch = [8, [* (message / status / space-notice)]]` from the server. A catch-up entry may
//!   come alone or in a non-empty `batch`.
//! - The mailboxes: `mailbox_entries` holds every entry, a `status` as well as a `message`, by
//!   `device_id` and `seq`.
//!
//! Expiry ("Mailbox"): a `message` expires at its storing time plus its envelope's `ttl`, capped by
//! the server's mailbox retention, a `status` at its adding time plus the retention, and a
//! `message` whose `v` is neither the connection's version nor adjacent to it when the server
//! reaches it while sending. The server finds expired entries by itself, deletes them and never
//! sends them, and adds `expired` for an envelope sent with `report` true. Besides the above:
//! - The setting `--mailbox-retention <SECONDS>`, also `SKALD_MAILBOX_RETENTION`; unset, there is
//!   no retention.
//! - The server keeps the times of envelopes, sends and entries in `timestamptz` columns of the
//!   tables `envelopes`, `sends` and `entries`, and reads them there: the tests move them back to
//!   let time pass.
//! - The server finds an expired entry within 30 seconds, with no connection of its device.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod expiring;
mod link;
mod scene;

mod acks;
mod catch_up;
mod connections;
mod expiry;
mod no_retention;
mod receipts;
mod versions;
