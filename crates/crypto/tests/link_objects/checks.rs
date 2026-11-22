//! Signing a link object with the issuing device's key over the label `skald link`, and the new
//! device's check of it (`membership.md`, "Linking a device" and "Link code", step 3 of the new
//! device): the object names this device, its issuer is the device whose certificate signed the
//! grant's certificate, and the signature over `["skald link", body]` verifies with that key.

use skald_crypto::{check_link, sign_link};
use skald_proto::{Cbor, LINK_LABEL, Link};

use crate::fields::{signed_cbor, unknown};
use crate::fixtures::{BROWSER, DESKTOP, LAPTOP, OTHER_LAPTOP, PHONE, device, key, public};
use crate::objects::Linked;

/// `linked`, decoded.
fn decoded(linked: &Linked) -> Link {
    Link::from_cbor(&linked.encoded()).expect("a fixture link object decodes")
}

/// Whether the desktop's check of `link`, with the laptop as the issuer of its certificate, passes.
fn desktop_accepts(link: &Link) -> bool {
    check_link(link, device(DESKTOP), &public(LAPTOP)).is_ok()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_label_of_a_link_object_is_skald_link() {
    assert_eq!(LINK_LABEL, "skald link");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_issuing_device_signs_a_link_object_over_its_body_and_the_link_label() {
    for linked in [
        Linked::desktop(),
        Linked::no_personal(),
        Linked::nothing_accepted(),
    ] {
        let body = linked.proto();
        let link = sign_link(&body, &key(LAPTOP)).expect("a link object of a valid body");
        assert_eq!(link.body(), &body);
        assert_eq!(link.body_bytes(), linked.body(), "the body, byte for byte");
        assert_eq!(link.signature(), &linked.signature(), "{linked:?}");
        assert_eq!(
            link.to_cbor(),
            linked.encoded(),
            "the object, byte for byte"
        );
        assert!(
            desktop_accepts(&link),
            "a signed link object passes its check"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_for_this_device_by_its_issuer_passes_the_check() {
    let later = Linked {
        later: unknown(),
        ..Linked::desktop()
    };
    for linked in [
        Linked::desktop(),
        Linked::no_personal(),
        Linked::nothing_accepted(),
        later,
    ] {
        assert!(desktop_accepts(&decoded(&linked)), "{linked:?}");
    }
    let phone = Linked {
        new: PHONE,
        issuer: DESKTOP,
        signer: DESKTOP,
        ..Linked::desktop()
    };
    assert!(check_link(&decoded(&phone), device(PHONE), &public(DESKTOP)).is_ok());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_for_another_device_fails_the_check() {
    let for_phone = Linked {
        new: PHONE,
        ..Linked::desktop()
    };
    assert!(
        !desktop_accepts(&decoded(&for_phone)),
        "a link for the phone"
    );
    assert!(
        check_link(&decoded(&Linked::desktop()), device(PHONE), &public(LAPTOP)).is_err(),
        "the desktop's link, checked by the phone"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_of_another_signer_or_issuer_fails_the_check() {
    let desktop = Linked::desktop();
    let cases = [
        (
            "signed by the browser, naming the laptop",
            Linked {
                signer: BROWSER,
                ..desktop.clone()
            },
        ),
        (
            "signed by another account's device, naming the laptop",
            Linked {
                signer: OTHER_LAPTOP,
                ..desktop.clone()
            },
        ),
        (
            "naming the browser as its issuer, signed by the laptop",
            Linked {
                issuer: BROWSER,
                ..desktop.clone()
            },
        ),
        (
            "by the browser, naming the browser",
            Linked {
                issuer: BROWSER,
                signer: BROWSER,
                ..desktop.clone()
            },
        ),
        (
            "signed by the new device itself",
            Linked {
                signer: DESKTOP,
                ..desktop.clone()
            },
        ),
    ];
    for (what, linked) in &cases {
        assert!(!desktop_accepts(&decoded(linked)), "a link object {what}");
    }
    // The issuer of the grant's certificate is the browser: the laptop's link does not pass.
    assert!(
        check_link(&decoded(&desktop), device(DESKTOP), &public(BROWSER)).is_err(),
        "the laptop's link object, when the browser issued the certificate"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_object_over_another_label_or_with_a_changed_body_fails_the_check() {
    let desktop = Linked::desktop();
    for label in [
        "skald anchors",
        "skald cert",
        "skald proof",
        "skald links",
        "skald roster",
    ] {
        let linked = Linked {
            label,
            ..desktop.clone()
        };
        assert!(!desktop_accepts(&decoded(&linked)), "over {label:?}");
    }
    let signature = desktop.signature();
    let changed = [
        Linked {
            personal: None,
            ..desktop.clone()
        },
        Linked::no_personal(),
        Linked {
            later: unknown(),
            ..desktop.clone()
        },
    ];
    for other in &changed {
        let swapped = Link::from_cbor(&signed_cbor(&other.body(), &signature))
            .expect("a link object with another body");
        assert!(
            !desktop_accepts(&swapped),
            "{other:?} under the desktop's signature"
        );
    }
    let mut flipped = signature;
    flipped[63] ^= 1;
    let flipped = Link::from_cbor(&signed_cbor(&desktop.body(), &flipped)).expect("a link object");
    assert!(!desktop_accepts(&flipped), "a changed signature");
}
