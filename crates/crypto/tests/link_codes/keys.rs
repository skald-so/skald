//! The request and grant keys: `HKDF-SHA256(salt = "", IKM = ISK, info = label, L = 16)` with the
//! labels `skald link request` and `skald link grant`; each seals one plaintext with
//! AEAD_AES_128_GCM, a nonce of 12 zero bytes and empty associated data, so a resend repeats the
//! same `sealed`. `docs/protocol/test-vectors.md` gives both keys of a fixed ISK.

use skald_crypto::Isk;

use crate::fixtures::{draft, hex, vector_section};

/// The fixed ISK: the draft's ISK_IR.
fn vector_isk() -> Isk {
    Isk::from_bytes(&hex(draft::ISK).try_into().unwrap())
}

/// The keys of the fixed ISK, computed apart from the crate.
const REQUEST_KEY: &str = "34ef61cde245a794a66fb3a2a1119e06";
const GRANT_KEY: &str = "f59082ca8b39050a140e5a40d4bc616b";
/// [`PLAINTEXT`] sealed with each, computed apart from the crate: ciphertext, then the tag.
const PLAINTEXT: &[u8] = b"skald link test";
const REQUEST_SEALED: &str = "1ac64ce7dc7b6554503c135dc7ce92240353fc905792016f23a2a16c7e79f0";
const GRANT_SEALED: &str = "d04b154ed6d577f0279c06f14cd70c537cc273da51bdb2cd700312b5337e81";

/// The section of the doc that gives the keys.
const SECTION: &str = "## Link code keys";

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_keys_of_the_fixed_isk_are_the_vectors() {
    let isk = vector_isk();
    assert_eq!(isk.request_key().to_bytes().as_slice(), hex(REQUEST_KEY));
    assert_eq!(isk.grant_key().to_bytes().as_slice(), hex(GRANT_KEY));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_give_the_keys_of_the_fixed_isk() {
    let section = vector_section(SECTION);
    for (what, value) in [
        ("the ISK", draft::ISK),
        ("the request key", REQUEST_KEY),
        ("the grant key", GRANT_KEY),
    ] {
        let digits: String = value.split_whitespace().collect();
        assert!(
            section.contains(&digits),
            "{SECTION:?} gives {what}: {digits}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_key_seals_with_a_zero_nonce_and_no_associated_data() {
    let isk = vector_isk();
    let request = isk.request_key().seal(PLAINTEXT);
    assert_eq!(request, hex(REQUEST_SEALED), "the request");
    assert_eq!(
        request.len(),
        PLAINTEXT.len() + 16,
        "the ciphertext and its tag"
    );
    let grant = isk.grant_key().seal(PLAINTEXT);
    assert_eq!(grant, hex(GRANT_SEALED), "the grant");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_resend_repeats_the_same_sealed() {
    let isk = vector_isk();
    assert_eq!(
        isk.request_key().seal(PLAINTEXT),
        vector_isk().request_key().seal(PLAINTEXT)
    );
    assert_eq!(
        isk.grant_key().seal(PLAINTEXT),
        vector_isk().grant_key().seal(PLAINTEXT)
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_request_and_grant_keys_differ_and_open_only_their_own() {
    let isk = vector_isk();
    assert_ne!(
        isk.request_key().to_bytes().as_slice(),
        isk.grant_key().to_bytes().as_slice()
    );
    let request = isk.request_key().seal(PLAINTEXT);
    let grant = isk.grant_key().seal(PLAINTEXT);
    assert_ne!(request, grant);
    let opened = isk.request_key().open(&request).expect("the request opens");
    assert_eq!(opened.as_slice(), PLAINTEXT);
    let opened = isk.grant_key().open(&grant).expect("the grant opens");
    assert_eq!(opened.as_slice(), PLAINTEXT);
    assert!(
        isk.grant_key().open(&request).is_err(),
        "the grant key opens no request"
    );
    assert!(
        isk.request_key().open(&grant).is_err(),
        "the request key opens no grant"
    );

    let mut other = hex(draft::ISK);
    other[0] ^= 1;
    let other = Isk::from_bytes(&other.try_into().unwrap());
    assert!(other.request_key().open(&request).is_err(), "another ISK");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_or_truncated_sealed_does_not_open() {
    let isk = vector_isk();
    let sealed = isk.request_key().seal(PLAINTEXT);
    for at in [0, PLAINTEXT.len(), sealed.len() - 1] {
        let mut changed = sealed.clone();
        changed[at] ^= 1;
        assert!(isk.request_key().open(&changed).is_err(), "changed at {at}");
    }
    for len in [0, 15, 16, sealed.len() - 1] {
        assert!(
            isk.request_key().open(&sealed[..len]).is_err(),
            "{len} bytes"
        );
    }
    let mut longer = sealed.clone();
    longer.push(0);
    assert!(isk.request_key().open(&longer).is_err(), "a byte more");
}
