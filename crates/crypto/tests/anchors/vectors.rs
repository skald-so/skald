//! The anchors object of `docs/protocol/test-vectors.md`, section "Anchors object": it decodes and
//! encodes back to the published bytes, its body in the deterministic encoding of its fields, and
//! names the signer's personal space and at least one anchor. `skald-proto`'s test of the vectors
//! checks its value and diagnostic notation.

use skald_proto::{Anchors, AnchorsBody, Cbor};

use crate::cbor::Value;
use crate::common::hex;
use crate::objects::spec_fields;

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The section's name.
const NAME: &str = "Anchors object";

/// The bytes of the section's `hex` block.
fn published() -> Vec<u8> {
    let section = DOC
        .split("\n## ")
        .find_map(|section| section.strip_prefix(&format!("{NAME}\n")))
        .unwrap_or_else(|| panic!("test-vectors.md has no section \"## {NAME}\""));
    let digits = section
        .split_once("```hex\n")
        .and_then(|(_, rest)| rest.split_once("\n```"))
        .map(|(digits, _)| digits)
        .unwrap_or_else(|| panic!("{NAME:?} has no hex block"));
    hex(digits)
}

/// The deterministic encoding of `body`'s spec fields.
fn encoding_of(body: &AnchorsBody) -> Vec<u8> {
    Value::Map(spec_fields(
        body.account,
        body.device,
        &body.anchors,
        body.personal,
    ))
    .encode()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_an_anchors_object_that_names_the_personal_space() {
    let encoded = published();
    let anchors = Anchors::from_cbor(&encoded).unwrap_or_else(|error| panic!("{NAME}: {error}"));
    assert_eq!(anchors.to_cbor(), encoded, "{NAME}, byte for byte");
    let body = anchors.body();
    assert_eq!(body.to_cbor(), anchors.body_bytes(), "{NAME}: the body");
    assert_eq!(
        encoding_of(body),
        anchors.body_bytes(),
        "{NAME}: the body holds only its spec fields, in the deterministic encoding"
    );
    assert!(body.personal.is_some(), "{NAME} names no personal space");
    assert!(!body.anchors.is_empty(), "{NAME} names no anchor");
}
