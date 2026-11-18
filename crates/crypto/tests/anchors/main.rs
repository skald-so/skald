//! Anchors objects (`docs/protocol/membership.md`, "Anchors"), natively and in WASM: their type in
//! `skald-proto`, encoded as "Signed objects" says and keeping their body's bytes; signing one with
//! a device's signing key over the label `skald anchors` in `skald-crypto`, and checking it with
//! that device's key; the anchors object of `docs/protocol/test-vectors.md`; and the field of the
//! personal space that the spec gives link and anchors objects.
//!
//! The interfaces, as these tests use them:
//! - `skald_proto::ANCHORS_LABEL`, `"skald anchors"`, and `skald_proto::Anchors`, the
//!   `Signed<AnchorsBody>` of an anchors object;
//! - `skald_proto::AnchorsBody`, with the public fields `account: AccountId`, `device: DeviceId`,
//!   `anchors: Vec<(SpaceId, [u8; 32])>`, each a space and the hash of a revision of it, in the
//!   order the body lists them, `personal: Option<SpaceId>`, the account's personal space, and
//!   `unknown: Unknown`;
//! - `skald_crypto::sign_anchors(&AnchorsBody, &SigningKey) -> Result<Anchors, _>`;
//! - `skald_crypto::check_anchors(&Anchors, &[u8; 32]) -> Result<(), _>`: `Ok` only when the key is
//!   the signing key of the object's `device` and the signature over `["skald anchors", body]`
//!   verifies with it.
//!
//! The fixtures encode anchors objects from their spec fields and sign them with ed25519-dalek,
//! apart from the crates under test.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
// The keys and devices of the tests of revocations; this target uses some of them.
#[allow(dead_code)]
#[path = "../revocations/fixtures.rs"]
mod fixtures;

mod encoding;
mod objects;
mod signing;
mod spec;
mod vectors;
