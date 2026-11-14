//! Storing sent envelopes (`docs/protocol/delivery.md`, "Send", "Mailbox" and "Order"): on a
//! connection authenticated as a device, the server answers each `send` with `stored` or
//! `send-refused`, stores each envelope once, as its bytes, in its recipient device's mailbox, in
//! the order it stores them, and refuses the others with their problem, storing nothing. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, with roster
//! revisions that the tests' devices sign and that list a second account.
//!
//! The interfaces, as these tests use them:
//! - The connection: a challenge from `POST /session/challenge` with the device session, then
//!   `/ws` with the first frame `auth`, as the tests of connections authenticated as a device say.
//! - The frames of "Frames": `send = [1, envelope: bstr .cbor envelope, report: bool]` from the
//!   device; `stored = [2, send-key, timestamp]` and `send-refused = [3, send-key, problem: tstr]`
//!   from the server, with `send-key = [msg_id, ? recipient]`.
//! - The mailboxes, as the database shows them: the table or view `mailbox_entries`, one row per
//!   entry, with the columns `device_id`, the device whose mailbox holds it, `seq`, an integer, and
//!   `envelope`, a `bytea` holding the bytes of a `message` entry's envelope.

#[path = "../common/mod.rs"]
mod common;

// The spaces tests' accounts and revisions; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;

mod fixture;
mod wire;

mod frames;
mod order;
mod refusals;
mod retries;
mod stored;
