//! A space's roster and policy: their types in `skald-proto`, and signing and checking revisions in
//! `skald-crypto`, the one implementation the client and the server share (`docs/protocol/
//! membership.md`, "Signed objects", "Roster" and "Policy"; `docs/protocol/README.md`, "Encoding").
//!
//! The fixtures encode revisions from their spec fields and sign them with ed25519-dalek, apart
//! from the crates under test: Ed25519 is deterministic, so signatures match byte for byte.

mod cbor;
mod checks;
mod encoding;
mod fixtures;
mod signing;
