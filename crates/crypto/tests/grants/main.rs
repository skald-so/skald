//! A service grant as a recipient checks it: checks 1 to 3 of `docs/protocol/README.md`, "Service
//! grant", in `skald-crypto`, which the client and the web app share, natively and in WASM.
//!
//! The space and its devices are those of the tests of rosters (`rosters/fixtures.rs`): the
//! revision the client accepted last has the admins alice and bob, the member carol and the service
//! agent `ci/deploy-bot`, bob's `service` device. The client under test is one of alice's devices,
//! alice's master key pinned; another account's master key comes from the revision. Grants are
//! encoded from their spec fields and signed with ed25519-dalek over `["skald grant", body]`, apart
//! from the crates under test.
//!
//! The interface, as these tests use it:
//! - `skald_crypto::check_grant(grant, (service, channel), roster, certs, (account, master), now)`:
//!   `Ok(())` when `grant`, in content from the device `service` in `channel`, passes the checks
//!   against `roster`, the latest revision the client accepted, the certificates the server returns
//!   `certs`, the client's own `account` and its pinned `master` key, at `now`, seconds since the
//!   Unix epoch;
//! - otherwise a `skald_crypto::GrantError`: `Service` when it names another device, `Channel` when
//!   it does not list the channel, `Signer(device)` when its signer is no admin's device in
//!   `roster`, `Signature` when its signature does not verify with the signer's certificate,
//!   `Expired` when `expires` has passed.

// The fixtures of the tests of rosters serve more than these tests use.
#![allow(dead_code)]

#[path = "../rosters/cbor.rs"]
mod cbor;
mod checks;
#[path = "../rosters/fixtures.rs"]
mod fixtures;
mod grant;
