//! The vectors of `docs/protocol/test-vectors.md`, each item of a section a list item
//! `- <name>: `<hex>``:
//! - "Queue key pair": `History key`, and the `Private key` and `Public key` of its epoch's queue
//!   key pair, as RFC 9180 serializes them;
//! - "Sealed task": the encoded `queued-task` in its `hex` block, sealed to `Public key`, the
//!   `pk` of "Queue key pair", with the ephemeral key pair `DeriveKeyPair(Ephemeral key)`;
//!   `Plaintext`, the encoded `task-plaintext` it seals, signed by `Signing key`, the Ed25519
//!   secret key of its `sender`;
//! - "Queue key object": the encoded `queue-key` in its `hex` block, its `key` the `pk` of "Queue
//!   key pair", signed by `Signing key`, the Ed25519 secret key of its `signer`.
//!
//! They are genuine: they derive, open and verify as the spec says apart from the crate, and the
//! crate derives, opens, checks, seals and signs them to the same bytes.

use hpke::Serializable as _;
use hpke::kem::X25519HkdfSha256;
use skald_crypto::{
    HistoryKey, SigningKey, check_queue_key, check_task, seal_task_with_ephemeral, sign_queue_key,
    verify,
};
use skald_proto::{Cbor, DeviceId, QueueKey, QueueMode, QueuedTask, Task, TaskPlaintext};

use crate::common::hex;
use crate::fixtures::{aad, claim, open, pair};
use crate::items::{QUEUE_KEY_LABEL, TASK_LABEL};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The lines of the section `## <name>`, up to the next section.
fn lines(name: &str) -> Vec<&'static str> {
    let heading = format!("## {name}");
    let mut lines = DOC.lines().skip_while(|line| line.trim_end() != heading);
    assert!(
        lines.next().is_some(),
        "test-vectors.md has no section {heading:?}"
    );
    lines.take_while(|line| !line.starts_with("## ")).collect()
}

/// The bytes of the list item `- <item>: `<hex>`` of the section `## <name>`.
fn item(name: &str, item: &str) -> Vec<u8> {
    let prefix = format!("- {item}: ");
    let digits = lines(name)
        .into_iter()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("{name:?} has no item {item:?}"));
    hex(digits.trim().trim_matches('`'))
}

fn item32(name: &str, item_name: &str) -> [u8; 32] {
    item(name, item_name)
        .try_into()
        .unwrap_or_else(|_| panic!("{name:?}: {item_name:?} is 32 bytes"))
}

/// The bytes of the `hex` block of the section `## <name>`.
fn block(name: &str) -> Vec<u8> {
    let digits: String = lines(name)
        .into_iter()
        .skip_while(|line| line.trim_end() != "```hex")
        .skip(1)
        .take_while(|line| line.trim_end() != "```")
        .collect();
    assert!(!digits.is_empty(), "{name:?} has no hex block");
    hex(&digits)
}

/// "Queue key pair": the history key and its pair's `sk` and `pk`.
fn queue_key_pair() -> ([u8; 16], [u8; 32], [u8; 32]) {
    let history = item("Queue key pair", "History key")
        .try_into()
        .expect("a 16-byte history key");
    (
        history,
        item32("Queue key pair", "Private key"),
        item32("Queue key pair", "Public key"),
    )
}

/// "Sealed task": the record, decoded and encoded, and its plaintext.
fn sealed_task() -> (QueuedTask, Vec<u8>, Vec<u8>) {
    let encoded = block("Sealed task");
    let record = QueuedTask::from_cbor(&encoded).expect("the published task decodes");
    (record, encoded, item("Sealed task", "Plaintext"))
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_queue_key_pair_derived_as_the_spec_says() {
    let (history, sk, pk) = queue_key_pair();

    assert_eq!(
        pair(&history),
        (sk, pk),
        "DeriveKeyPair(HKDF-SHA256(\"\", history key, \"skald queue\", 32))"
    );
    assert_eq!(
        HistoryKey::from_bytes(&history).queue_key().public_key(),
        pk,
        "the crate derives the published pk"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_sealed_task_that_opens_as_the_spec_says() {
    let (_, sk, pk) = queue_key_pair();
    let (record, encoded, plaintext) = sealed_task();
    let signing = SigningKey::from_bytes(&item32("Sealed task", "Signing key"));
    let ephemeral = item("Sealed task", "Ephemeral key");

    assert_eq!(record.to_cbor(), encoded, "the record, byte for byte");
    assert_eq!(
        item32("Sealed task", "Public key"),
        pk,
        "sealed to the pk of \"Queue key pair\""
    );
    let (_, ephemeral_pk) = <X25519HkdfSha256 as hpke::Kem>::derive_keypair(&ephemeral);
    assert_eq!(
        record.enc.as_slice(),
        ephemeral_pk.to_bytes().as_slice(),
        "enc is the public key of DeriveKeyPair(Ephemeral key)"
    );

    let signed = TaskPlaintext::from_cbor(&plaintext).expect("the published plaintext decodes");
    assert_eq!(signed.to_cbor(), plaintext, "the plaintext, byte for byte");
    let body = signed.body();
    let opened = open(
        &sk,
        &record.enc,
        &aad(body.channel, record.queue, record.task, record.epoch),
        &record.ct,
    )
    .expect("the record opens with the epoch's sk, info and aad as the spec says");
    assert_eq!(opened, plaintext, "it seals the published plaintext");

    verify(
        &signing.public_key(),
        TASK_LABEL,
        &[signed.body_bytes()],
        signed.signature(),
    )
    .expect("signed by the signing key over [\"skald task\", body]");
    assert_eq!(
        body.sender,
        DeviceId::from_signing_key(&signing.public_key()),
        "the sender is the signing key's device"
    );
    assert_eq!(
        (body.queue, body.task, &body.sched),
        (record.queue, record.task, &record.sched),
        "the body's queue, task and sched are the record's"
    );
    assert_eq!(body.content.counter, None, "a task has no counter");
    let task = Task::from_cbor(body.content.unknown.get(11).expect("a task body"))
        .expect("the task body decodes");
    assert!(
        task.request
            .contains(&format!("\"messageId\":\"{}\"", record.task)),
        "the request's messageId is the task in text form"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_crate_opens_the_sealed_task_vector_and_seals_its_plaintext_to_it() {
    let (history, _, pk) = queue_key_pair();
    let (record, encoded, plaintext) = sealed_task();
    let signing = SigningKey::from_bytes(&item32("Sealed task", "Signing key"));
    let key = HistoryKey::from_bytes(&history).queue_key();
    let signed = TaskPlaintext::from_cbor(&plaintext).expect("the published plaintext decodes");
    let channel = signed.body().channel;

    let opened = key
        .open_task(&record, channel, record.queue)
        .expect("the crate opens the published task");
    assert_eq!(opened.to_cbor(), plaintext, "the published plaintext");
    check_task(&opened, &signing.public_key()).expect("the crate checks its signature");
    let mode = signed.body().mode;
    let body = key
        .check_claim(
            &claim(&record, mode),
            channel,
            record.queue,
            &signing.public_key(),
        )
        .expect("the published task passes checks 1 to 3");
    assert_eq!(&body, signed.body());
    let other = match mode {
        QueueMode::AtLeastOnce => QueueMode::AtMostOnce,
        QueueMode::AtMostOnce => QueueMode::AtLeastOnce,
    };
    assert!(
        key.check_claim(
            &claim(&record, other),
            channel,
            record.queue,
            &signing.public_key()
        )
        .is_err(),
        "a claim of the other mode fails check 3"
    );

    let sealed = seal_task_with_ephemeral(
        &signed,
        record.epoch,
        &pk,
        &item32("Sealed task", "Ephemeral key"),
    )
    .expect("the crate seals the published plaintext");
    assert_eq!(
        sealed.to_cbor(),
        encoded,
        "the published record, byte for byte"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_queue_key_object_signed_as_the_spec_says() {
    let (_, _, pk) = queue_key_pair();
    let encoded = block("Queue key object");
    let signing = SigningKey::from_bytes(&item32("Queue key object", "Signing key"));

    let object = QueueKey::from_cbor(&encoded).expect("the published object decodes");

    assert_eq!(object.to_cbor(), encoded, "the object, byte for byte");
    let body = object.body();
    assert_eq!(body.key, pk, "its key is the pk of \"Queue key pair\"");
    assert_eq!(
        body.signer,
        DeviceId::from_signing_key(&signing.public_key()),
        "the signer is the signing key's device"
    );
    verify(
        &signing.public_key(),
        QUEUE_KEY_LABEL,
        &[object.body_bytes()],
        object.signature(),
    )
    .expect("signed by the signing key over [\"skald queue key\", body]");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_crate_checks_the_queue_key_object_vector_and_signs_its_body_to_it() {
    let encoded = block("Queue key object");
    let signing = SigningKey::from_bytes(&item32("Queue key object", "Signing key"));
    let object = QueueKey::from_cbor(&encoded).expect("the published object decodes");

    check_queue_key(&object, &signing.public_key()).expect("the crate checks the published object");
    let signed = sign_queue_key(object.body(), &signing).expect("the crate signs the body");
    assert_eq!(
        signed.to_cbor(),
        encoded,
        "the published object, byte for byte"
    );
}
