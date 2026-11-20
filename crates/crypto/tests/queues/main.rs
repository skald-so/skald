//! Queues and tasks, natively and in WASM: `docs/protocol/queues.md`. The types of `skald-proto`
//! for its CBOR items, `queued-task`, `sched`, `task-plaintext` with its `task-body`, the `task` and
//! `task-result` bodies, `claim` and `queue-key` with its body, decode into their spec fields,
//! `queue-mode` and `outcome` among them, and encode back byte for byte, keeping fields of later
//! versions and the signed bodies' bytes. Malformed and truncated input is refused, never a panic.
//!
//! The fixtures encode the items apart from the crates under test, with the tests' CBOR writer,
//! from their spec fields, and sign them with the tests' devices.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../common/devices.rs"]
mod devices;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
mod fixtures;
mod items;
mod refused;
