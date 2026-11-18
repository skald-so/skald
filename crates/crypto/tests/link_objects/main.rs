//! Link objects and the request and grant bodies of a link code's run
//! (`docs/protocol/membership.md`, "Linking a device" and "Link code"), natively and in WASM: their
//! types in `skald-proto`, encoded as "Signed objects" and `README.md`, "Encoding", say and keeping
//! the bytes they came in; signing a link object with the issuing device's key over the label
//! `skald link` in `skald-crypto`, and the new device's check of it; and the link object of
//! `docs/protocol/test-vectors.md`.
//!
//! The interfaces, as these tests use them:
//! - `skald_proto::LINK_LABEL`, `"skald link"`, and `skald_proto::Link`, the `Signed<LinkBody>` of a
//!   link object;
//! - `skald_proto::LinkBody`, with the public fields `device: DeviceId`, the new device, `issuer:
//!   DeviceId`, `anchors: Vec<(SpaceId, [u8; 32])>`, each a space and the hash of a revision of it,
//!   in the order the body lists them, `personal: Option<SpaceId>` and `unknown: Unknown`;
//! - `skald_proto::LinkRequestBody`, a `link-request-body`, with `kind: DeviceKind`, `signing: [u8;
//!   32]`, `hpke: [u8; 32]`, `name: String` and `unknown: Unknown`;
//! - `skald_proto::LinkGrantBody`, a `link-grant-body`, with `master: [u8; 32]`, `cert: Vec<u8>`,
//!   the encoded `cert-body` as it came, which must decode as one, `signature: [u8; 64]`, `link:
//!   Link` and `unknown: Unknown`;
//! - each of them `Cbor`: `to_cbor` and `from_cbor`;
//! - `skald_crypto::sign_link(&LinkBody, &SigningKey) -> Result<Link, _>`;
//! - `skald_crypto::check_link(&Link, DeviceId, &[u8; 32]) -> Result<(), _>`: `Ok` only when the
//!   object's `device` is the given device, its `issuer` derives from the given key, the signing key
//!   of the issuer's certificate, and the signature over `["skald link", body]` verifies with it.
//!
//! The fixtures encode the objects from their spec fields and sign them with ed25519-dalek, apart
//! from the crates under test.

mod bodies;
mod checks;
mod encoding;
mod objects;
mod vectors;

// The keys and devices of the tests of revocations, and the shared CBOR and field helpers; this
// target uses some of each.
#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
#[allow(dead_code)]
#[path = "../revocations/fixtures.rs"]
mod fixtures;
