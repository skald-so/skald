//! A task body signed by its sender over `["skald task", body]`, and verified with a given device
//! key: of claim check 2, the signature.

use skald_crypto::{check_task, sign_task, verify};
use skald_proto::{Cbor, TaskBody, TaskPlaintext};

use crate::cbor::{Value, bytes};
use crate::devices::{device_id, signing_key};
use crate::fixtures::{assert_refused, body_fields, replaced};
use crate::items::{
    ADDRESSEE, ENQUEUED, Ids, QUEUE_KEY_LABEL, SENDER, TASK_LABEL, encoded, signed, with_unknown,
};

/// The public signing key of device `seed`.
fn public(seed: u8) -> [u8; 32] {
    signing_key(seed).public_key()
}

/// The task plaintext of `body` signed by device `seed` under `label`.
fn plaintext(body: &[u8], seed: u8, label: &str) -> TaskPlaintext {
    TaskPlaintext::from_cbor(&signed(body, seed, label)).expect("the fixture plaintext decodes")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_crate_signs_a_task_body_by_its_sender_under_skald_task() {
    let body = encoded(&body_fields(&Ids::new()));
    let decoded = TaskBody::from_cbor(&body).expect("the fixture body decodes");

    let plaintext = sign_task(&decoded, &signing_key(SENDER)).expect("the crate signs the body");

    assert_eq!(plaintext.body_bytes(), body, "the body, byte for byte");
    verify(
        &public(SENDER),
        TASK_LABEL,
        &[plaintext.body_bytes()],
        plaintext.signature(),
    )
    .expect("signed by the sender over [\"skald task\", body]");
    assert_eq!(
        plaintext.to_cbor(),
        signed(&body, SENDER, TASK_LABEL),
        "the plaintext, byte for byte"
    );
    check_task(&plaintext, &public(SENDER)).expect("the crate's signature checks");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_signed_by_its_sender_checks_with_its_key_fields_of_later_versions_included() {
    let ids = Ids::new();
    for body in [
        encoded(&body_fields(&ids)),
        encoded(&with_unknown(body_fields(&ids))),
    ] {
        check_task(&plaintext(&body, SENDER, TASK_LABEL), &public(SENDER))
            .expect("a task its sender signed checks with the sender's key");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_signed_under_another_label_does_not_check() {
    let body = encoded(&body_fields(&Ids::new()));
    for label in [QUEUE_KEY_LABEL, "skald hpke", "skald tasks", ""] {
        assert_refused(
            check_task(&plaintext(&body, SENDER, label), &public(SENDER)),
            &format!("a task signed under {label:?}"),
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_does_not_check_with_another_signing_key() {
    let ids = Ids::new();
    let body = encoded(&body_fields(&ids));

    assert_refused(
        check_task(&plaintext(&body, ADDRESSEE, TASK_LABEL), &public(SENDER)),
        "a task its sender names but another device signed, checked with the sender's key",
    );
    assert_refused(
        check_task(&plaintext(&body, SENDER, TASK_LABEL), &public(ADDRESSEE)),
        "a task its sender signed, checked with another device's key",
    );
    let other_sender = encoded(&replaced(
        &body_fields(&ids),
        1,
        bytes(device_id(ADDRESSEE).as_bytes()),
    ));
    assert_refused(
        check_task(
            &plaintext(&other_sender, SENDER, TASK_LABEL),
            &public(SENDER),
        ),
        "a task that names another sender than the signing key's device",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_body_or_signature_does_not_check() {
    let ids = Ids::new();
    let fields = body_fields(&ids);
    let original = plaintext(&encoded(&fields), SENDER, TASK_LABEL);

    let later = encoded(&replaced(&fields, 7, Value::Uint(ENQUEUED + 1)));
    let changed_body =
        TaskPlaintext::from_parts(later, *original.signature()).expect("the changed body decodes");
    assert_refused(
        check_task(&changed_body, &public(SENDER)),
        "a body changed after its signature",
    );

    let mut signature = *original.signature();
    signature[10] ^= 0x01;
    let changed_signature = TaskPlaintext::from_parts(original.body_bytes().to_vec(), signature)
        .expect("the body decodes");
    assert_refused(
        check_task(&changed_signature, &public(SENDER)),
        "a changed signature",
    );
}
