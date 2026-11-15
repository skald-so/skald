//! The cloud limits on delivery (`docs/dev/architecture.md`, "Cloud limits"): server settings give
//! the largest message, the messages a space stores per calendar month and the devices with a
//! certificate an account may have. Unset, none applies, as a self-hosted server applies no plan
//! limits. Against a `skald-server` as in the tests of storing sent envelopes, whose fixture and
//! frames these tests share; every HTTP response is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The settings, each also an environment variable:
//! - `--message-size-limit <BYTES>`, `SKALD_MESSAGE_SIZE_LIMIT`: the largest payload of an
//!   envelope, in bytes. An envelope whose payload is longer is refused with `too-large`
//!   (`docs/protocol/delivery.md`, "Send"); the message size is the payload length
//!   (`docs/protocol/README.md`, "Envelope").
//! - `--monthly-messages-limit <N>`, `SKALD_MONTHLY_MESSAGES_LIMIT`: the messages a space stores
//!   per calendar month, in UTC. A message is a sending device's `msg_id`: it counts once, whatever
//!   its envelopes, a retry does not count, and neither does a refused one. A message past its
//!   space's number for the month is refused with `quota-exceeded`.
//! - `--devices-limit <N>`, `SKALD_DEVICES_LIMIT`: the devices with a certificate an account may
//!   have. `POST /devices` for an account that has that many is refused with `403` and the problem
//!   `https://skald.so/problems/devices-limit`, and registers nothing. Devices without a
//!   certificate and `service` devices do not count.
//!
//! What the tests set up and read in the server's database, besides what the tests of storing sent
//! envelopes read:
//! - `devices`: one row per device, as the tests of sending in the CLI write them.
//! - The server counts a month's messages by the times and months it keeps in `timestamptz` and
//!   `date` columns of the tables `envelopes`, `sends`, `entries` and, if it has one,
//!   `message_counts`: the tests move a count to the month before by moving each of them one month
//!   back.

// The frames and fixture of the tests of storing sent envelopes, and the spaces tests' accounts and
// revisions they build on; this binary uses some of them.
#[allow(dead_code)]
#[path = "../spaces/api.rs"]
mod api;
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../mailbox/fixture.rs"]
mod fixture;
#[allow(dead_code)]
#[path = "../spaces/rosters.rs"]
mod rosters;
#[allow(dead_code)]
#[path = "../mailbox/wire.rs"]
mod wire;

mod limits;

mod devices;
mod message_size;
mod monthly_messages;
