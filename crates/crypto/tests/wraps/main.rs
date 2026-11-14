//! Wraps of an epoch's history key, natively and in WASM: `docs/protocol/membership.md`,
//! "Account" and "Wraps". The recovery key's HPKE key pair is `DeriveKeyPair(recovery key)` of
//! DHKEM(X25519, HKDF-SHA256); a member device seals an epoch's key to a `browser` device's `hpke`
//! key or to an account's recovery HPKE key and signs the wrap; a reader opens it with its HPKE
//! key, verifies the signature with the certificate of `signer` and reads the history from it.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/devices.rs"]
mod devices;
mod errors;
#[path = "../common/fields.rs"]
mod fields;
mod fixtures;
mod history;
mod objects;
mod recovery;
#[path = "../common/rfc9180.rs"]
mod rfc9180;
mod wrapping;
