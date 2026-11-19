//! The link object of `docs/protocol/test-vectors.md`, section "Link object": it decodes and
//! encodes back to the published bytes, its body in the deterministic encoding of its spec fields,
//! for a new device other than its issuer, with at least one anchor and a personal space.

use skald_proto::{Cbor, Link};

use crate::cbor::{Value, bytes};
use crate::common::hex;
use crate::objects::starts_value;

const VECTORS: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The hex digits of the first `hex` block under the heading `## Link object`.
fn link_object_hex() -> String {
    let mut lines = VECTORS.lines().skip_while(|line| *line != "## Link object");
    assert!(
        lines.next().is_some(),
        "test-vectors.md has no section \"## Link object\""
    );
    let mut lines = lines
        .take_while(|line| !line.starts_with("## "))
        .skip_while(|line| *line != "```hex")
        .skip(1);
    let digits: String = lines
        .by_ref()
        .take_while(|line| !line.starts_with("```"))
        .collect();
    assert!(!digits.is_empty(), "the link object has no hex block");
    digits
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_link_object_that_encodes_back_byte_for_byte() {
    let published = hex(&link_object_hex());
    let link = Link::from_cbor(&published).unwrap_or_else(|error| panic!("Link object: {error}"));
    assert_eq!(link.to_cbor(), published, "the link object, byte for byte");

    let body = link.body();
    assert_eq!(body.to_cbor(), link.body_bytes(), "its body");
    let mut fields = vec![
        (1, bytes(body.device.as_bytes())),
        (2, bytes(body.issuer.as_bytes())),
        (3, starts_value(&body.anchors)),
    ];
    if let Some(personal) = body.personal {
        fields.push((4, bytes(personal.as_bytes())));
    }
    assert_eq!(
        Value::Map(fields).encode(),
        link.body_bytes(),
        "the body holds only its spec fields, in the deterministic encoding"
    );
    assert_ne!(body.device, body.issuer, "the new device is not its issuer");
    assert!(!body.anchors.is_empty(), "the link object names no anchor");
    assert!(
        body.personal.is_some(),
        "the link object names no personal space"
    );
}
