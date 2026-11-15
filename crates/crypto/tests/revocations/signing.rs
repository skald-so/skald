//! Signing a revocation with a device's signing key or the master key over the label
//! `skald revocation` (`membership.md`, "Signed objects" and "Revocation").

use skald_crypto::{sign_revocation, verify};
use skald_proto::{Cbor, DeviceId, NonEmpty, REVOCATION_LABEL, RevocationBody, Unknown};

use crate::fixtures::*;

fn ids(seeds: &[u8]) -> Option<NonEmpty<DeviceId>> {
    NonEmpty::from_vec(seeds.iter().copied().map(device).collect())
}

/// The body of `rev` as a client builds it.
fn body_of(rev: &Rev) -> RevocationBody {
    RevocationBody {
        account: rev.account,
        device: device(rev.device),
        keep: ids(&rev.keep),
        signer: rev.signer.map(device),
        timestamp: TIMESTAMP,
        also: ids(&rev.also),
        unknown: Unknown::default(),
    }
}

/// Checks that signing the body of `rev` with `rev.key` gives `rev`, byte for byte.
fn assert_signs(rev: &Rev) {
    let body = body_of(rev);
    let revocation = sign_revocation(&body, &key(rev.key)).expect("a revocation of a valid body");
    assert_eq!(revocation.body(), &body);
    assert_eq!(body.to_cbor(), rev.body(), "the body, byte for byte");
    assert_eq!(revocation.body_bytes(), rev.body());
    assert_eq!(revocation.signature(), &rev.signature());
    assert_eq!(
        revocation.to_cbor(),
        rev.bytes(),
        "the revocation, byte for byte"
    );
    assert!(
        verify(
            &public(rev.key),
            LABEL,
            &[revocation.body_bytes()],
            revocation.signature()
        )
        .is_ok()
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_label_of_a_revocation_is_skald_revocation() {
    assert_eq!(REVOCATION_LABEL, "skald revocation");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_signs_a_revocation_over_its_encoded_body_and_the_revocation_label() {
    assert_signs(&Rev::by(LAPTOP, DESKTOP));
    assert_signs(&Rev::by(BROWSER, DESKTOP).keep(&[PHONE, TABLET]));
    assert_signs(&Rev::by(DESKTOP, DESKTOP));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_master_key_signs_a_revocation_over_its_encoded_body_and_the_revocation_label() {
    assert_signs(&Rev::by_master(DESKTOP));
    assert_signs(
        &Rev::by_master(DESKTOP)
            .keep(&[PHONE])
            .also(&[BROWSER, SERVICE]),
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_the_client_signs_counts() {
    let certs = certs();
    let master = public(MASTER);
    let by_laptop = sign_revocation(
        &body_of(&Rev::by(LAPTOP, DESKTOP).keep(&[PHONE])),
        &key(LAPTOP),
    )
    .unwrap();
    let by_master = sign_revocation(
        &body_of(&Rev::by_master(BROWSER).also(&[TABLET])),
        &key(MASTER),
    )
    .unwrap();
    let revocations = [by_laptop, by_master];
    let status = |seed| {
        skald_crypto::check_valid_device(&master, &certs, &revocations, device(seed)).is_ok()
    };
    for seed in [DESKTOP, TABLET, BROWSER] {
        assert!(!status(seed), "{} is revoked", name(seed));
    }
    for seed in [LAPTOP, PHONE, PHONE_BROWSER, SERVICE, RECOVERED] {
        assert!(status(seed), "{} is valid", name(seed));
    }
}
