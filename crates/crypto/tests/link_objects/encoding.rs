//! The link type of `skald-proto`: `membership.md`, "Linking a device", encoded as "Signed objects"
//! and `README.md`, "Encoding", say. A link object keeps its body's bytes as it got them, unknown
//! fields included; malformed and truncated input is refused, never a panic.

use skald_proto::{Cbor, Link, LinkBody};

use crate::cbor::{Value, bytes, text};
use crate::fields::{signed_cbor, truncated_and_longer, unknown, with, without};
use crate::fixtures::device;
use crate::objects::{INITECH, Linked, starts_value};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_decodes_into_its_spec_fields() {
    for linked in [
        Linked::desktop(),
        Linked::no_personal(),
        Linked::nothing_accepted(),
    ] {
        let link = Link::from_cbor(&linked.encoded()).expect("a link object");
        let body = link.body();
        assert_eq!(body.device, device(linked.new), "{linked:?}");
        assert_eq!(body.issuer, device(linked.issuer), "{linked:?}");
        assert_eq!(body.anchors, linked.starts, "the anchors, in order");
        assert_eq!(body.personal, linked.personal, "{linked:?}");
        assert_eq!(link.body_bytes(), linked.body());
        assert_eq!(link.signature(), &linked.signature());
        assert_eq!(link.to_cbor(), linked.encoded(), "{linked:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_body_encodes_to_the_bytes_of_its_spec_fields() {
    for linked in [
        Linked::desktop(),
        Linked::no_personal(),
        Linked::nothing_accepted(),
    ] {
        let built = linked.proto();
        assert_eq!(built.to_cbor(), linked.body(), "{linked:?}");
        assert_eq!(LinkBody::from_cbor(&linked.body()).as_ref(), Ok(&built));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_with_fields_of_later_versions_round_trips_byte_for_byte() {
    for base in [Linked::desktop(), Linked::nothing_accepted()] {
        let linked = Linked {
            later: unknown(),
            ..base
        };
        let body = LinkBody::from_cbor(&linked.body()).expect("a link body with later fields");
        let mut kept: Vec<i128> = body.unknown.keys().collect();
        kept.sort_unstable();
        assert_eq!(kept, [-3, 24], "the later fields: {:?}", body.unknown);
        assert_eq!(body.to_cbor(), linked.body(), "the body, byte for byte");

        let link = Link::from_cbor(&linked.encoded()).expect("a link object with later fields");
        assert_eq!(link.body(), &body);
        assert_eq!(link.body_bytes(), linked.body());
        assert_eq!(
            link.to_cbor(),
            linked.encoded(),
            "the object, byte for byte"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_link_bodies_are_refused() {
    let fields = Linked::desktop().fields();
    let one_anchor = |pair: Vec<Value>| Value::Array(vec![Value::Array(pair)]);
    let initech = || bytes(INITECH.as_bytes());
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("without the new device", without(&fields, 1)),
        ("without the issuer", without(&fields, 2)),
        ("without anchors", without(&fields, 3)),
        (
            "a new device of 17 bytes",
            with(&fields, 1, bytes(&[0x21; 17])),
        ),
        ("an issuer that is text", with(&fields, 2, text("laptop"))),
        (
            "anchors that are a map",
            with(&fields, 3, Value::Map(Vec::new())),
        ),
        (
            "an anchor of one item",
            with(&fields, 3, one_anchor(vec![initech()])),
        ),
        (
            "an anchor of three items",
            with(
                &fields,
                3,
                one_anchor(vec![initech(), bytes(&[0xe5; 32]), bytes(&[0xe5; 32])]),
            ),
        ),
        (
            "a revision hash of 16 bytes",
            with(&fields, 3, starts_value(&[]).with_anchor(initech(), 16)),
        ),
        (
            "a revision hash of 64 bytes",
            with(&fields, 3, starts_value(&[]).with_anchor(initech(), 64)),
        ),
        (
            "an anchored space of 8 bytes",
            with(
                &fields,
                3,
                starts_value(&[]).with_anchor(bytes(&[0x6a; 8]), 32),
            ),
        ),
        (
            "a personal space that is a number",
            with(&fields, 4, Value::Uint(9)),
        ),
        (
            "a personal space of 32 bytes",
            with(&fields, 4, bytes(&[0x6c; 32])),
        ),
        (
            "a body that is an array",
            Value::Array(vec![initech(), initech()]).encode(),
        ),
    ];
    for (what, body) in &cases {
        assert!(LinkBody::from_cbor(body).is_err(), "a link body {what}");
        assert!(
            Link::from_cbor(&signed_cbor(body, &[0x7b; 64])).is_err(),
            "a link object whose body is {what}"
        );
    }
}

/// An anchors array grown by one anchor, used to make malformed ones.
trait WithAnchor {
    /// This array with one more anchor of `space` and a revision hash of `len` bytes.
    fn with_anchor(self, space: Value, len: usize) -> Value;
}

impl WithAnchor for Value {
    fn with_anchor(self, space: Value, len: usize) -> Value {
        let Value::Array(mut listed) = self else {
            panic!("anchors are an array");
        };
        listed.push(Value::Array(vec![space, bytes(&vec![0xe5; len])]));
        Value::Array(listed)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_link_objects_are_refused() {
    let linked = Linked::desktop();
    for input in truncated_and_longer(&linked.encoded()) {
        assert!(Link::from_cbor(&input).is_err(), "{input:?}");
    }
    let body = linked.body();
    let signature = linked.signature();
    let shapes = [
        (
            "a signature of 32 bytes",
            signed_cbor(&body, &signature[..32]),
        ),
        (
            "a signature of 96 bytes",
            signed_cbor(&body, &[signature, signature].concat()[..96]),
        ),
        ("only a body", Value::Array(vec![bytes(&body)]).encode()),
        (
            "a body that is a map in place of its bytes",
            Value::Array(vec![Value::Map(linked.fields()), bytes(&signature)]).encode(),
        ),
        (
            "a proof after the signature, as a certificate has",
            Value::Array(vec![bytes(&body), bytes(&signature), bytes(&signature)]).encode(),
        ),
    ];
    for (what, input) in &shapes {
        assert!(Link::from_cbor(input).is_err(), "{what}");
    }
}
