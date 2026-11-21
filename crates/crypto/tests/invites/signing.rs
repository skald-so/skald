//! Signing an invite with an admin's device key over the label `skald invite`, and an acceptance
//! with the invite key over the label `skald acceptance`, and checking each with its key
//! (`membership.md`, "Signed objects" and "Invite").

use skald_crypto::{InviteSecret, check_acceptance, check_invite, sign_acceptance, sign_invite};
use skald_proto::{ACCEPTANCE_LABEL, Acceptance, Cbor, INVITE_LABEL, Invite};

use crate::fields::{signed_cbor, unknown};
use crate::fixtures::{DESKTOP, LAPTOP, key, public};
use crate::objects::{Acc, By, DESIGN, Inv, OTHER_SECRET, Object, SECRET, assert_is};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_labels_are_skald_invite_and_skald_acceptance() {
    assert_eq!(INVITE_LABEL, "skald invite");
    assert_eq!(ACCEPTANCE_LABEL, "skald acceptance");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_signs_an_invite_over_its_encoded_body_and_the_invite_label() {
    for inv in [Inv::of_member(), Inv::of_admin_to_no_channel()] {
        let body = inv.proto_body();
        let invite = sign_invite(&body, &key(LAPTOP)).expect("an invite of a valid body");
        assert_eq!(invite.body(), &body);
        assert_is(&invite, &inv);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_invite_key_signs_an_acceptance_over_its_encoded_body_and_the_acceptance_label() {
    for acc in [Acc::of_newcomer(), Acc::with_other_handle()] {
        let body = acc.proto_body();
        let invite_key = InviteSecret::from_bytes(&SECRET).invite_key();
        let acceptance =
            sign_acceptance(&body, &invite_key).expect("an acceptance of a valid body");
        assert_eq!(acceptance.body(), &body);
        assert_is(&acceptance, &acc);
    }
}

fn invite(inv: &Inv) -> Invite {
    Invite::from_cbor(&inv.bytes()).expect("a fixture invite decodes")
}

fn acceptance(acc: &Acc) -> Acceptance {
    Acceptance::from_cbor(&acc.bytes()).expect("a fixture acceptance decodes")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_checks_with_the_key_of_its_signer() {
    let with_later_fields = Inv {
        extra: unknown(),
        ..Inv::of_member()
    };
    let by_desktop = Inv {
        signer: DESKTOP,
        by: By::Device(DESKTOP),
        channels: vec![(DESIGN, true)],
        ..Inv::of_member()
    };
    for inv in [
        Inv::of_member(),
        Inv::of_admin_to_no_channel(),
        with_later_fields,
    ] {
        assert!(
            check_invite(&invite(&inv), &public(LAPTOP)).is_ok(),
            "{inv:?}"
        );
    }
    assert!(check_invite(&invite(&by_desktop), &public(DESKTOP)).is_ok());

    let signed = sign_invite(&Inv::of_member().proto_body(), &key(LAPTOP)).expect("an invite");
    assert!(check_invite(&signed, &public(LAPTOP)).is_ok());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_acceptance_checks_with_the_invite_key() {
    let with_later_fields = Acc {
        extra: unknown(),
        ..Acc::of_newcomer()
    };
    for acc in [
        Acc::of_newcomer(),
        Acc::with_other_handle(),
        with_later_fields,
    ] {
        assert!(
            check_acceptance(&acceptance(&acc), &By::Secret(SECRET).public()).is_ok(),
            "{acc:?}"
        );
    }

    let invite_key = InviteSecret::from_bytes(&SECRET).invite_key();
    let signed =
        sign_acceptance(&Acc::of_newcomer().proto_body(), &invite_key).expect("an acceptance");
    assert!(check_acceptance(&signed, &invite_key.public_key()).is_ok());
    assert!(check_acceptance(&signed, &Inv::of_member().key).is_ok());
}

/// The labels of the other signed objects, and near misses of `label`.
fn other_labels(near: [&'static str; 2]) -> Vec<&'static str> {
    let mut labels = vec![
        "skald roster",
        "skald anchors",
        "skald link",
        "skald revocation",
        "skald cert",
    ];
    labels.extend(near);
    labels
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_over_another_label_or_by_another_key_fails_its_check() {
    let laptop = Inv::of_member();
    let mut cases: Vec<(String, Invite, [u8; 32])> =
        other_labels(["skald acceptance", "skald invites"])
            .into_iter()
            .map(|label| {
                let inv = Inv {
                    label,
                    ..laptop.clone()
                };
                (format!("over {label:?}"), invite(&inv), public(LAPTOP))
            })
            .collect();
    let by_desktop = invite(&Inv {
        by: By::Device(DESKTOP),
        ..laptop.clone()
    });
    cases.push((
        "by another device's key, checked with its signer's".to_owned(),
        by_desktop.clone(),
        public(LAPTOP),
    ));
    cases.push((
        "by another device's key, checked with that key".to_owned(),
        by_desktop,
        public(DESKTOP),
    ));
    cases.push((
        "checked with another device's key".to_owned(),
        invite(&laptop),
        public(DESKTOP),
    ));
    let by_invite_key = invite(&Inv {
        by: By::Secret(SECRET),
        ..laptop.clone()
    });
    cases.push((
        "by its own invite key, checked with its signer's".to_owned(),
        by_invite_key.clone(),
        public(LAPTOP),
    ));
    cases.push((
        "by its own invite key, checked with that key".to_owned(),
        by_invite_key,
        By::Secret(SECRET).public(),
    ));
    let mut broken = laptop.signature();
    broken[17] ^= 0x04;
    cases.push((
        "with a broken signature".to_owned(),
        Invite::from_cbor(&signed_cbor(&laptop.body(), &broken)).unwrap(),
        public(LAPTOP),
    ));
    let other_body = Inv {
        expires: laptop.expires + 86_400,
        ..laptop.clone()
    };
    cases.push((
        "with the signature of another body".to_owned(),
        Invite::from_cbor(&signed_cbor(&other_body.body(), &laptop.signature())).unwrap(),
        public(LAPTOP),
    ));
    for (what, invite, key) in &cases {
        assert!(check_invite(invite, key).is_err(), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_acceptance_over_another_label_or_by_another_key_fails_its_check() {
    let newcomer = Acc::of_newcomer();
    let invite_key = By::Secret(SECRET).public();
    let mut cases: Vec<(String, Acceptance, [u8; 32])> =
        other_labels(["skald invite", "skald accept"])
            .into_iter()
            .map(|label| {
                let acc = Acc {
                    label,
                    ..newcomer.clone()
                };
                (format!("over {label:?}"), acceptance(&acc), invite_key)
            })
            .collect();
    let by_other_secret = acceptance(&Acc {
        by: By::Secret(OTHER_SECRET),
        ..newcomer.clone()
    });
    cases.push((
        "by another invite key, checked with the invite's".to_owned(),
        by_other_secret,
        invite_key,
    ));
    cases.push((
        "checked with another invite key".to_owned(),
        acceptance(&newcomer),
        By::Secret(OTHER_SECRET).public(),
    ));
    let by_device = acceptance(&Acc {
        by: By::Device(LAPTOP),
        ..newcomer.clone()
    });
    cases.push((
        "by the signer's device key, checked with the invite key".to_owned(),
        by_device,
        invite_key,
    ));
    let mut broken = newcomer.signature();
    broken[40] ^= 0x10;
    cases.push((
        "with a broken signature".to_owned(),
        Acceptance::from_cbor(&signed_cbor(&newcomer.body(), &broken)).unwrap(),
        invite_key,
    ));
    let other_body = Acc {
        handle: "mallory",
        ..newcomer.clone()
    };
    cases.push((
        "with the signature of another body".to_owned(),
        Acceptance::from_cbor(&signed_cbor(&other_body.body(), &newcomer.signature())).unwrap(),
        invite_key,
    ));
    for (what, acceptance, key) in &cases {
        assert!(check_acceptance(acceptance, key).is_err(), "{what}");
    }
}
