//! Presence values, natively and in WASM: `docs/protocol/agents.md`, "Presence". Their types in
//! `skald-proto` keep unknown fields and the signed body's bytes; `skald-crypto` derives an epoch's
//! presence key from its history key, seals a body signed by its device, padded, under a random
//! nonce and the spec's associated data, and opens a value with the reader checks that need nothing
//! of the reader's state; the sealed value of `docs/protocol/test-vectors.md`.
//!
//! The fixtures encode, sign and seal values apart from the crates under test: the tests' CBOR
//! writer, HKDF-SHA256 and AES-128-GCM as the spec gives them, and Ed25519, which is deterministic,
//! so a sealed plaintext matches byte for byte.

#[path = "../rosters/cbor.rs"]
mod cbor;
mod checks;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../common/devices.rs"]
mod devices;
mod encoding;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
mod fixtures;
mod sealing;
mod vectors;
