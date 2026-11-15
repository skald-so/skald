//! Server instances waking each other (`docs/dev/architecture.md`, "Network"): several
//! `skald-server` instances on one database wake each other through PostgreSQL `LISTEN/NOTIFY`.
//! An instance that adds mailbox entries, stores a roster revision or ends a session notifies the
//! others, which push the entries, send the `space-notice` or close that session's connections at
//! once, whichever instance a device is connected to. An instance whose listening connection drops
//! listens again, then sends its connections what they missed. Expiry on several instances adds one
//! `expired` per entry, and an ack racing an expiry gives exactly one of `delivered` and `expired`.
//! Against instances on the database of the tests of storing sent envelopes, whose fixture and
//! frames these tests share.
//!
//! The interfaces, as these tests use them, besides those of storing and delivering a mailbox:
//! - Each instance is a `skald-server` on the same database, with the same public URL and sign-in.
//! - At once: within a second of the `stored`, the ack or the revision that causes it.
//! - `space-notice = [9, space: id]` ("Frames"), sent for a new roster revision to each connection
//!   of the devices of the accounts that get the space.
//! - Signing out, `DELETE /session`, closes the session's connections with code 1008 and the reason
//!   `session-ended` ("Connecting").
//! - An instance's listening connection: its backend in PostgreSQL whose last statement is `LISTEN`
//!   or `UNLISTEN`, as `pg_stat_activity` shows it.
//! - The mailbox retention: the server setting `--mailbox-retention <SECONDS>`; unset, the instance
//!   expires nothing. The server keeps the times of envelopes, sends and mailbox entries in the
//!   `timestamptz` columns of the tables `envelopes`, `sends` and `entries`, which the tests move
//!   back; it finds an expired entry by itself within 30 s.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

// The fixture and frames of the tests of storing sent envelopes, and the entries of the tests of
// delivering a mailbox; this binary uses some of them.
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../mailbox_delivery/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod instances;
mod proxy;
mod watch;

mod expiry;
mod listening;
mod sessions;
mod wake_ups;
