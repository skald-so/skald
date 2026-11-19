//! A sender's trust level, as a recipient device sets it from the sending device's valid
//! certificate, its `account` and `kind`, and the roster of the recipient's space, never from the
//! content (`docs/protocol/membership.md`, "Trust levels"): `own` for a `client` or `browser`
//! device of the recipient's account, `peer` for one of another account the roster lists as a
//! member, `service` for a `service` device; `own` is above `peer`, which is above `service`.
//!
//! The level, as these tests use it: `skald_crypto::trust_level(recipient, cert, roster)` of the
//! recipient's account, the sending device's valid certificate and the body of the recipient
//! space's roster gives a `skald_proto::TrustLevel`, `Own`, `Peer` or `Service`, ordered so, or
//! none for a `client` or `browser` device of another account the roster does not list.
//!
//! The accounts, devices and revisions are those of the tests of rosters (`rosters/fixtures.rs`):
//! the space `acme` with alice, bob and carol; dave's account is outside it.

// The fixtures serve the tests of rosters too.
#![allow(dead_code)]

#[path = "../rosters/cbor.rs"]
mod cbor;
#[path = "../rosters/fixtures.rs"]
mod fixtures;

use skald_crypto::trust_level;
use skald_proto::{AccountId, Cert, Roster, TrustLevel};

use crate::fixtures::*;

/// The certificate of device `seed` among the fixtures'.
fn cert(seed: u8) -> Cert {
    (certs().into_iter())
        .find(|cert| cert.body().device == device(seed))
        .unwrap_or_else(|| panic!("no certificate of device {seed}"))
}

/// The level that a device of `recipient` gives a message from device `seed`, with `roster` the
/// roster of its space.
fn level(recipient: AccountId, seed: u8, roster: &Roster) -> Option<TrustLevel> {
    trust_level(recipient, &cert(seed), roster.body())
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_client_or_browser_device_of_the_recipient_s_account_is_own() {
    let roster = current().roster();
    assert_eq!(level(ALICE, ALICE_LAPTOP, &roster), Some(TrustLevel::Own));
    assert_eq!(level(ALICE, ALICE_BROWSER, &roster), Some(TrustLevel::Own));
    assert_eq!(level(BOB, BOB_LAPTOP, &roster), Some(TrustLevel::Own));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_client_device_of_another_account_the_roster_lists_is_peer() {
    let roster = current().roster();
    assert_eq!(level(ALICE, BOB_LAPTOP, &roster), Some(TrustLevel::Peer));
    assert_eq!(level(ALICE, CAROL_LAPTOP, &roster), Some(TrustLevel::Peer));
    assert_eq!(level(BOB, ALICE_LAPTOP, &roster), Some(TrustLevel::Peer));
    assert_eq!(level(BOB, ALICE_BROWSER, &roster), Some(TrustLevel::Peer));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_service_device_is_service_on_the_recipient_s_own_account_too() {
    let roster = current().roster();
    assert_eq!(level(ALICE, BOB_CI, &roster), Some(TrustLevel::Service));
    assert_eq!(level(BOB, BOB_CI, &roster), Some(TrustLevel::Service));
    assert_eq!(
        level(ALICE, ALICE_SERVICE, &roster),
        Some(TrustLevel::Service)
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_of_an_account_the_roster_does_not_list_has_no_level() {
    let roster = current().roster();
    assert_eq!(level(ALICE, DAVE_LAPTOP, &roster), None);
    // Revision 1 lists alice alone: bob's device is no peer of hers there.
    let alone = revision_one().roster();
    assert_eq!(level(ALICE, BOB_LAPTOP, &alone), None);
    assert_eq!(level(ALICE, ALICE_LAPTOP, &alone), Some(TrustLevel::Own));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn own_is_above_peer_which_is_above_service() {
    assert!(TrustLevel::Own > TrustLevel::Peer);
    assert!(TrustLevel::Peer > TrustLevel::Service);
    assert!(TrustLevel::Own > TrustLevel::Service);
}
