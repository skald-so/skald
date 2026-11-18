//! Signing an anchors object with a device's signing key over the label `skald anchors`, and
//! checking it with the key of its `device` (`membership.md`, "Signed objects" and "Anchors").

use skald_crypto::{check_anchors, sign_anchors};
use skald_proto::{ANCHORS_LABEL, Anchors, Cbor};

use crate::fields::{signed_cbor, unknown};
use crate::fixtures::{DESKTOP, key, public};
use crate::objects::{Anch, PERSONAL};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_label_of_an_anchors_object_is_skald_anchors() {
    assert_eq!(ANCHORS_LABEL, "skald anchors");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_signs_an_anchors_object_over_its_encoded_body_and_the_anchors_label() {
    for anch in [Anch::of_laptop(), Anch::without_personal(), Anch::empty()] {
        let body = anch.proto_body();
        let anchors =
            sign_anchors(&body, &key(anch.key)).expect("an anchors object of a valid body");
        assert_eq!(anchors.body(), &body);
        assert_eq!(anchors.body_bytes(), anch.body(), "the body, byte for byte");
        assert_eq!(anchors.signature(), &anch.signature());
        assert_eq!(anchors.to_cbor(), anch.bytes(), "the object, byte for byte");
    }
}

/// `anch`, decoded.
fn decoded(anch: &Anch) -> Anchors {
    Anchors::from_cbor(&anch.bytes()).expect("a fixture anchors object decodes")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_anchors_object_checks_with_the_key_of_its_device() {
    let with_later_fields = Anch {
        extra: unknown(),
        ..Anch::of_laptop()
    };
    for anch in [Anch::of_laptop(), Anch::empty(), with_later_fields] {
        assert!(
            check_anchors(&decoded(&anch), &public(anch.key)).is_ok(),
            "{anch:?}"
        );
    }
    let signed = sign_anchors(&Anch::of_laptop().proto_body(), &key(Anch::of_laptop().key))
        .expect("an anchors object");
    assert!(check_anchors(&signed, &public(Anch::of_laptop().key)).is_ok());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_anchors_object_over_another_label_or_by_another_key_fails_its_check() {
    let laptop = Anch::of_laptop();
    let mut cases: Vec<(String, Anchors, [u8; 32])> = [
        "skald revocation",
        "skald link",
        "skald roster",
        "skald cert",
        "skald anchor",
    ]
    .into_iter()
    .map(|label| {
        let anch = Anch {
            label,
            ..laptop.clone()
        };
        (
            format!("over {label:?}"),
            decoded(&anch),
            public(laptop.key),
        )
    })
    .collect();
    let by_desktop = decoded(&Anch {
        key: DESKTOP,
        ..laptop.clone()
    });
    cases.push((
        "by another device's key, checked with its device's".to_owned(),
        by_desktop.clone(),
        public(laptop.key),
    ));
    cases.push((
        "by another device's key, checked with that key".to_owned(),
        by_desktop,
        public(DESKTOP),
    ));
    cases.push((
        "checked with another device's key".to_owned(),
        decoded(&laptop),
        public(DESKTOP),
    ));
    let mut broken = laptop.signature();
    broken[17] ^= 0x04;
    cases.push((
        "with a broken signature".to_owned(),
        Anchors::from_cbor(&signed_cbor(&laptop.body(), &broken)).unwrap(),
        public(laptop.key),
    ));
    let other_body = Anch {
        personal: None,
        anchors: vec![(PERSONAL, [0xc3; 32])],
        ..laptop.clone()
    };
    cases.push((
        "with the signature of another body".to_owned(),
        Anchors::from_cbor(&signed_cbor(&other_body.body(), &laptop.signature())).unwrap(),
        public(laptop.key),
    ));
    for (what, anchors, key) in &cases {
        assert!(check_anchors(anchors, key).is_err(), "{what}");
    }
}
