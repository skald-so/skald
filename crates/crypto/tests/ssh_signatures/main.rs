//! SSH signatures of device keys, natively and in WASM: a certificate's `ssh` and `github`, the
//! GitHub user whose key made it, in `skald-proto`, as `docs/protocol/membership.md`, "Device
//! certificate", says; checking `ssh` against a list of OpenSSH public keys in `skald-crypto`; and
//! the certificate of `docs/protocol/test-vectors.md`, "Certificate with an SSH signature".
//!
//! `ssh` holds the binary form of OpenSSH's PROTOCOL.sshsig: the base64 of its armored text,
//! decoded. The fixed signatures were made by `ssh-keygen -Y sign -n skald.so` (OpenSSH 10.2) with
//! keys made for these tests alone, over the encoded `["skald ssh", account, device, signing]` of
//! the fixture certificate, the label a text string and the rest byte strings, as
//! `docs/protocol/README.md`, "Conventions", encodes signed data.

#[path = "../rosters/cbor.rs"]
mod cbor;
mod checks;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
mod encoding;
mod fixtures;
mod vectors;
