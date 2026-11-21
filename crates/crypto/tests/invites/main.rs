//! Invites, acceptances and invite links (`docs/protocol/membership.md`, "Invite"), natively and in
//! WASM: their types in `skald-proto`, encoded as "Signed objects" says and keeping their body's
//! bytes; in `skald-crypto`, the invite's secret and the invite key derived from it, an invite
//! signed and checked with a device's key over the label `skald invite`, an acceptance with the
//! invite key over `skald acceptance`, and the link made from the server's web address, the invite
//! ID, the secret and a revision's hash and read back into them; and the vectors of
//! `docs/protocol/test-vectors.md`.
//!
//! The interfaces, as these tests use them:
//! - `skald_proto::INVITE_LABEL`, `"skald invite"`, and `skald_proto::Invite`, the
//!   `Signed<InviteBody>` of an invite;
//! - `skald_proto::InviteBody`, with the public fields `invite: InviteId`, `space: SpaceId`, `key:
//!   [u8; 32]`, the invite key, `role: Role`, `channels: Vec<(ChannelId, bool)>`, each a channel
//!   and whether the newcomer reads its earlier history, in the body's order, `signer: DeviceId`,
//!   `expires: u64`, in seconds since the Unix epoch, and `unknown: Unknown`;
//! - `skald_proto::ACCEPTANCE_LABEL`, `"skald acceptance"`, and `skald_proto::Acceptance`, the
//!   `Signed<AcceptanceBody>` of an acceptance;
//! - `skald_proto::AcceptanceBody`, with the public fields `invite: InviteId`, `account:
//!   AccountId`, `master: [u8; 32]`, `handle: Name`, a valid name as a roster's handles are, and
//!   `unknown: Unknown`;
//! - each of them `Cbor`: `to_cbor` and `from_cbor`;
//! - `skald_crypto::InviteSecret`, 32 bytes: `generate()`, `from_bytes(&[u8; 32])`, `to_bytes()`
//!   as `Zeroizing<[u8; 32]>`, and `invite_key() -> SigningKey`, the invite key pair;
//! - `skald_crypto::sign_invite(&InviteBody, &SigningKey) -> Result<Invite, _>`, by the device in
//!   `signer`;
//! - `skald_crypto::check_invite(&Invite, &[u8; 32]) -> Result<(), _>`: `Ok` only when the key is
//!   the signing key of the invite's `signer` and the signature over `["skald invite", body]`
//!   verifies with it;
//! - `skald_crypto::sign_acceptance(&AcceptanceBody, &SigningKey) -> Result<Acceptance, _>`, by
//!   the invite key;
//! - `skald_crypto::check_acceptance(&Acceptance, &[u8; 32]) -> Result<(), _>`: `Ok` only when the
//!   signature over `["skald acceptance", body]` verifies with the key, the invite's `key`;
//! - `skald_crypto::InviteLink`, with the public fields `web: String`, the server's web address as
//!   an origin such as `https://skald.example`, `invite: InviteId`, `secret: InviteSecret` and
//!   `revision: [u8; 32]`; `shown(&self)`, the link as text that derefs to `str`, and
//!   `InviteLink::parse(&str) -> Result<InviteLink, _>`, its error `Debug`.
//!
//! The fixtures encode the objects from their spec fields, derive invite keys with HKDF-SHA256 and
//! sign with ed25519-dalek, apart from the crates under test.

// The keys and devices of the tests of revocations, and the shared CBOR and field helpers; this
// target uses some of each.
#![allow(dead_code)]

mod encoding;
mod keys;
mod links;
mod objects;
mod signing;
mod vectors;

#[path = "../rosters/cbor.rs"]
mod cbor;
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/fields.rs"]
mod fields;
#[path = "../revocations/fixtures.rs"]
mod fixtures;
