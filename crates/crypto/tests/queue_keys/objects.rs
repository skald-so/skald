//! A queue key object signed by its `signer` over `["skald queue key", body]`, and verified with a
//! given device key: "Queue key object".

use skald_crypto::{HistoryKey, check_queue_key, sign_queue_key, verify};
use skald_proto::{Cbor, QueueKey, QueueKeyBody};

use crate::cbor::{Value, bytes};
use crate::devices::{device_id, signing_key};
use crate::fixtures::{HISTORY, OTHER_HISTORY, assert_refused, replaced};
use crate::items::{
    EPOCH, Ids, SENDER, SIGNER, TASK_LABEL, encoded, queue_key_body_fields, signed, with_unknown,
};

/// The public signing key of device `seed`.
fn public(seed: u8) -> [u8; 32] {
    signing_key(seed).public_key()
}

/// The fields of the object of a channel's [`EPOCH`] whose history key is [`HISTORY`], signed by
/// [`SIGNER`].
fn body_fields(ids: &Ids) -> Vec<(i64, Value)> {
    let pk = HistoryKey::from_bytes(&HISTORY).queue_key().public_key();
    replaced(&queue_key_body_fields(ids), 3, bytes(&pk))
}

/// The queue key object of `body` signed by device `seed` under `label`.
fn object(body: &[u8], seed: u8, label: &str) -> QueueKey {
    QueueKey::from_cbor(&signed(body, seed, label)).expect("the fixture object decodes")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_crate_signs_a_queue_key_object_by_its_signer_under_skald_queue_key() {
    let ids = Ids::new();
    let body = encoded(&body_fields(&ids));
    let decoded = QueueKeyBody::from_cbor(&body).expect("the fixture body decodes");

    let signed_object =
        sign_queue_key(&decoded, &signing_key(SIGNER)).expect("the crate signs the body");

    assert_eq!(signed_object.body_bytes(), body, "the body, byte for byte");
    verify(
        &public(SIGNER),
        "skald queue key",
        &[signed_object.body_bytes()],
        signed_object.signature(),
    )
    .expect("signed by the signer over [\"skald queue key\", body]");
    assert_eq!(
        signed_object.to_cbor(),
        signed(&body, SIGNER, "skald queue key"),
        "the object, byte for byte"
    );
    check_queue_key(&signed_object, &public(SIGNER)).expect("the crate's signature checks");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_object_signed_by_its_signer_checks_with_its_key_fields_of_later_versions_included() {
    let ids = Ids::new();
    for body in [
        encoded(&body_fields(&ids)),
        encoded(&with_unknown(body_fields(&ids))),
    ] {
        check_queue_key(&object(&body, SIGNER, "skald queue key"), &public(SIGNER))
            .expect("an object its signer signed checks with the signer's key");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_object_signed_under_another_label_does_not_check() {
    let body = encoded(&body_fields(&Ids::new()));
    for label in [TASK_LABEL, "skald queue", "skald history wrap", ""] {
        assert_refused(
            check_queue_key(&object(&body, SIGNER, label), &public(SIGNER)),
            &format!("an object signed under {label:?}"),
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_object_does_not_check_with_another_signing_key() {
    let ids = Ids::new();
    let body = encoded(&body_fields(&ids));

    assert_refused(
        check_queue_key(&object(&body, SENDER, "skald queue key"), &public(SIGNER)),
        "an object its signer names but another device signed, checked with the signer's key",
    );
    assert_refused(
        check_queue_key(&object(&body, SIGNER, "skald queue key"), &public(SENDER)),
        "an object its signer signed, checked with another device's key",
    );
    let mut fields = body_fields(&ids);
    fields.retain(|(key, _)| *key != 4);
    fields.push((4, bytes(device_id(SENDER).as_bytes())));
    let other_signer = object(&encoded(&fields), SIGNER, "skald queue key");
    assert_refused(
        check_queue_key(&other_signer, &public(SIGNER)),
        "an object that names another signer than the signing key's device",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_object_with_a_changed_body_or_signature_does_not_check() {
    let fields = body_fields(&Ids::new());
    let original = object(&encoded(&fields), SIGNER, "skald queue key");
    let other_pk = HistoryKey::from_bytes(&OTHER_HISTORY)
        .queue_key()
        .public_key();

    for (what, changed) in [
        ("another key", replaced(&fields, 3, bytes(&other_pk))),
        (
            "another epoch",
            replaced(&fields, 2, Value::Uint(EPOCH + 1)),
        ),
    ] {
        let swapped = QueueKey::from_parts(encoded(&changed), *original.signature())
            .expect("the changed body decodes");
        assert_refused(
            check_queue_key(&swapped, &public(SIGNER)),
            &format!("a body with {what} after its signature"),
        );
    }

    let mut signature = *original.signature();
    signature[63] ^= 0x01;
    let changed_signature =
        QueueKey::from_parts(original.body_bytes().to_vec(), signature).expect("the body decodes");
    assert_refused(
        check_queue_key(&changed_signature, &public(SIGNER)),
        "a changed signature",
    );
}
