//! The revocations of `docs/protocol/test-vectors.md`: one by a device and one by the master key,
//! which decode and encode back to the published bytes. `skald-proto`'s test of the vectors checks
//! their values and diagnostic notation.

use skald_proto::{Cbor, Revocation, RevocationBody};

use crate::common::hex;

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The bytes of the `hex` block in the section `## <name>` of the document.
fn published(name: &str) -> Vec<u8> {
    let heading = format!("## {name}");
    let mut lines = DOC.lines().skip_while(|line| *line != heading);
    assert!(
        lines.next().is_some(),
        "test-vectors.md has no section {heading:?}"
    );
    let section: Vec<&str> = lines.take_while(|line| !line.starts_with("## ")).collect();
    let block: String = section
        .iter()
        .skip_while(|line| **line != "```hex")
        .skip(1)
        .take_while(|line| **line != "```")
        .copied()
        .collect();
    assert!(!block.is_empty(), "{heading:?} has no hex block");
    hex(&block)
}

/// The published revocation `name`, checked to decode and encode back byte for byte.
fn vector(name: &str) -> Revocation {
    let encoded = published(name);
    let revocation =
        Revocation::from_cbor(&encoded).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(revocation.to_cbor(), encoded, "{name}, byte for byte");
    let body = RevocationBody::from_cbor(revocation.body_bytes()).unwrap();
    assert_eq!(body.to_cbor(), revocation.body_bytes(), "{name}: the body");
    revocation
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_revocation_by_a_device() {
    let revocation = vector("Revocation by a device");
    let body = revocation.body();
    assert!(body.signer.is_some(), "a device signs it");
    assert!(body.keep.is_some(), "it keeps a device");
    assert_eq!(body.also, None, "a device's revocation has no also");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_revocation_by_the_master_key() {
    let revocation = vector("Revocation by the master key");
    let body = revocation.body();
    assert_eq!(body.signer, None, "the master key signs it");
    assert!(body.also.is_some(), "it keeps devices revoked");
}
