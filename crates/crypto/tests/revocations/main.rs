//! Revocations, the one implementation the client and the server share, natively and in WASM:
//! their type in `skald-proto`, encoded as `docs/protocol/membership.md`, "Signed objects" says,
//! signing them in `skald-crypto`, and which of them count and what they revoke as "Revocation"
//! says, in checks 3 and 4 of "Device checks"; and the revocations of
//! `docs/protocol/test-vectors.md`.
//!
//! The fixtures encode revocations from their spec fields and sign them with ed25519-dalek, apart
//! from the crates under test: Ed25519 is deterministic, so signatures match byte for byte.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
mod counting;
mod encoding;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
mod fixtures;
mod ignored;
mod signing;
mod vectors;
mod voiding;
