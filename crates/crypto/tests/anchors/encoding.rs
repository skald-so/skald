//! The anchors type of `skald-proto`: `membership.md`, "Anchors", encoded as "Signed objects" and
//! `README.md`, "Encoding", say. An anchors object keeps its body's bytes as it got them, unknown
//! fields included. Malformed and truncated input is refused, never a panic.

use skald_proto::{Anchors, AnchorsBody, Cbor};

use crate::cbor::{Value, bytes, text};
use crate::fields::{
    assert_signed_round_trip, signed_cbor, truncated_and_longer, unknown, with, without,
};
use crate::fixtures::device;
use crate::objects::{ACME, Anch};

/// The tests' anchors objects without fields of later versions.
fn plain() -> [Anch; 3] {
    [Anch::of_laptop(), Anch::without_personal(), Anch::empty()]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_anchors_object_decodes_into_its_spec_fields() {
    for anch in plain() {
        let anchors = Anchors::from_cbor(&anch.bytes()).unwrap();
        let body = anchors.body();
        assert_eq!(body.account, anch.account);
        assert_eq!(body.device, device(anch.device));
        assert_eq!(body.anchors, anch.anchors, "the anchors, in order");
        assert_eq!(body.personal, anch.personal);
        assert_eq!(anchors.body_bytes(), anch.body());
        assert_eq!(anchors.signature(), &anch.signature());
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_anchors_body_encodes_to_its_spec_bytes() {
    for anch in plain() {
        let body = anch.proto_body();
        assert_eq!(body.to_cbor(), anch.body(), "{anch:?}");
        assert_eq!(AnchorsBody::from_cbor(&anch.body()), Ok(body), "{anch:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_anchors_object_round_trips_byte_for_byte_with_its_unknown_fields() {
    for anch in [Anch::of_laptop(), Anch::empty()] {
        let anch = Anch {
            extra: unknown(),
            ..anch
        };
        let body = AnchorsBody::from_cbor(&anch.body()).unwrap();
        assert_eq!(body.to_cbor(), anch.body(), "the body, byte for byte");
        assert_eq!(body.unknown.keys().count(), 2, "{:?}", body.unknown);
        for key in [-3, 24] {
            assert!(body.unknown.get(key).is_some(), "no field {key}");
        }
        assert_signed_round_trip(&anch.body(), &body);

        let anchors = Anchors::from_cbor(&anch.bytes()).unwrap();
        assert_eq!(anchors.body(), &body);
        assert_eq!(anchors.body_bytes(), anch.body());
        assert_eq!(anchors.to_cbor(), anch.bytes(), "the object, byte for byte");
    }
}

/// Checks that `body`, described as `what`, decodes neither alone nor in an anchors object.
fn assert_refused(what: &str, body: &[u8]) {
    assert!(AnchorsBody::from_cbor(body).is_err(), "{what}");
    assert!(
        Anchors::from_cbor(&signed_cbor(body, &[0x5a; 64])).is_err(),
        "an anchors object with {what}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_anchors_bodies_are_refused() {
    let fields = Anch::of_laptop().fields();
    let anchor = |space: Vec<u8>, revision: Vec<u8>| {
        Value::Array(vec![Value::Array(vec![
            Value::Bytes(space),
            Value::Bytes(revision),
        ])])
    };
    let space = ACME.as_bytes().to_vec();
    let cases = [
        ("no account", without(&fields, 1)),
        ("no device", without(&fields, 2)),
        ("no anchors", without(&fields, 3)),
        ("an account that is text", with(&fields, 1, text("acme"))),
        ("a device of 15 bytes", with(&fields, 2, bytes(&[0x11; 15]))),
        (
            "anchors that are no array",
            with(&fields, 3, Value::Uint(3)),
        ),
        (
            "an anchor without its revision",
            with(
                &fields,
                3,
                Value::Array(vec![Value::Array(vec![bytes(&space)])]),
            ),
        ),
        (
            "an anchor with a third item",
            with(
                &fields,
                3,
                Value::Array(vec![Value::Array(vec![
                    bytes(&space),
                    bytes(&[0xa1; 32]),
                    Value::Uint(1),
                ])]),
            ),
        ),
        (
            "a revision hash of 31 bytes",
            with(&fields, 3, anchor(space.clone(), vec![0xa1; 31])),
        ),
        (
            "a revision hash of 33 bytes",
            with(&fields, 3, anchor(space.clone(), vec![0xa1; 33])),
        ),
        (
            "an anchor's space of 15 bytes",
            with(&fields, 3, anchor(vec![0x5a; 15], vec![0xa1; 32])),
        ),
        (
            "an anchor that is a map",
            with(
                &fields,
                3,
                Value::Array(vec![Value::Map(vec![(1, bytes(&space))])]),
            ),
        ),
        (
            "a personal space that is a number",
            with(&fields, 4, Value::Uint(4)),
        ),
        (
            "a personal space of 17 bytes",
            with(&fields, 4, bytes(&[0x5c; 17])),
        ),
        (
            "a body that is an array",
            Value::Array(vec![bytes(&space)]).encode(),
        ),
    ];
    for (what, body) in &cases {
        assert_refused(what, body);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_anchors_objects_are_refused() {
    let anch = Anch::of_laptop();
    for input in truncated_and_longer(&anch.bytes()) {
        assert!(Anchors::from_cbor(&input).is_err(), "{input:?}");
    }
    let body = anch.body();
    for (what, input) in [
        ("a signature of 63 bytes", signed_cbor(&body, &[0x5a; 63])),
        ("a signature of 65 bytes", signed_cbor(&body, &[0x5a; 65])),
        (
            "a body that is a map, not a byte string",
            Value::Array(vec![anch.value(), bytes(&anch.signature())]).encode(),
        ),
        ("no signature", Value::Array(vec![bytes(&body)]).encode()),
        (
            "a third item",
            Value::Array(vec![
                bytes(&body),
                bytes(&anch.signature()),
                bytes(&anch.signature()),
            ])
            .encode(),
        ),
    ] {
        assert!(Anchors::from_cbor(&input).is_err(), "{what}");
    }
}
