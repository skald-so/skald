//! Link codes, natively and in WASM (`docs/protocol/membership.md`, "Link code"): the code, its
//! shown form and its entry; CPACE-RISTR255-SHA512 as initiator and responder, checked against the
//! test vectors of draft-irtf-cfrg-cpace-21, Appendix B.3, and against `pakery-cpace`, another
//! implementation of the draft; the request and grant keys of a run's ISK, with the vector of
//! `docs/protocol/test-vectors.md`; runs of a new device without a session, with their pairing;
//! and the link messages in `skald-proto`.
//!
//! The interfaces, as these tests use them, in `skald-crypto`:
//! - `LinkCode::generate()`: a fresh code. `LinkCode::from_entry(&str)`: an entry read as "Link
//!   code" says, `Err` unless it is 6 characters of the alphabet. `Display`: its shown form.
//! - `LinkCode::initiate(&self, account, &session)`: the new device's side, A, of a run with this
//!   code: its state, a `CpaceInitiator`, and its share Ya. `LinkCode::respond(&self, account,
//!   &session, &share)`: the issuing device's side, B, given A's share: its share Yb and the ISK.
//!   PRS, CI, sid and the empty ADa and ADb as "Link code" says; `session` is 16 bytes.
//! - `CpaceInitiator::start(prs, ci, sid, ad)`: A's state and share Ya with a fresh scalar;
//!   `CpaceInitiator::start_with_scalar(&scalar, prs, ci, sid, ad)` with the scalar given, 32
//!   bytes little endian as the draft prints them. `finish(self, &share, ad)`: the ISK, given B's
//!   share Yb and ADb.
//! - `CpaceResponder::respond(prs, ci, sid, ad, &share, peer_ad)`: B's share Yb and the ISK with a
//!   fresh scalar, given its ADb and A's share Ya and ADa; `CpaceResponder::respond_with_scalar(
//!   &scalar, prs, ci, sid, ad, &share, peer_ad)` with the scalar given.
//! - Shares are 32 bytes. `finish` and the responses are `Err` for a share that does not decode or
//!   a K that is the identity.
//! - `LinkPairing::generate()`, `LinkPairing::from_entry(&str)`, `as_str()` and `Display`: a run's
//!   pairing, 6 characters. `LinkEntry::from_entry(&str)`: `Code(LinkCode)` for 6 characters,
//!   `Paired { pairing, code }` for 12. `LinkCode::shown_with(&self, &pairing)`: the 12 characters
//!   in three groups of four. `LinkCode::initiate_paired(&self, &pairing, &session)` and
//!   `LinkCode::respond_paired(&self, &pairing, &session, &share)`: `initiate` and `respond` with
//!   the pairing's CI.
//! - `Isk::from_bytes(&[u8; 64])`, `Isk::to_bytes()`: the ISK and its 64 bytes.
//!   `Isk::request_key()` and `Isk::grant_key()`: its `LinkKey`s. `LinkKey::to_bytes()`: its 16
//!   bytes. `LinkKey::seal(self, plaintext)`: `sealed`, the key spent. `LinkKey::open(sealed)`: the
//!   plaintext, `Err` for bytes the key did not seal.
//!
//! In `skald-proto`: `LinkMessage`, a `Cbor` type with the variants `Hello { session, share }`,
//! `Reply { session, share }`, `Request { session, sealed }`, `Grant { session, sealed }` and
//! `Abort { session }`, `session` 16 bytes, `share` 32 and `sealed` a `Vec<u8>`.

#[path = "../common/mod.rs"]
mod common;
mod fixtures;

mod codes;
mod cross_check;
mod keys;
mod messages;
mod pairings;
mod runs;
mod vectors;
