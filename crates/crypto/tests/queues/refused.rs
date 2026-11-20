//! Input that breaks the CDDL of `docs/protocol/queues.md` or the encoding rules of `README.md`,
//! "Encoding", is refused, never a panic: a missing required field, a field of another type or
//! size, an `enc` not of 32 bytes, an `idempotency_key` empty or over 32 bytes, an `epoch`,
//! `generation` or `attempt` of 0, an unknown `queue-mode` or `outcome`, truncated and malformed
//! bytes.

use skald_proto::{
    Cbor, Claim, DecodeError, QueueKey, QueueKeyBody, QueuedTask, Sched, Task, TaskBody,
    TaskPlaintext, TaskResult,
};

use crate::cbor::{Value, bytes, text};
use crate::fields::{signed_cbor, with, without};
use crate::fixtures::*;

/// The items of the spec, one per variant.
#[derive(Debug, Clone, Copy)]
enum Item {
    QueuedTask,
    Sched,
    TaskBody,
    TaskPlaintext,
    Task,
    TaskResult,
    Claim,
    QueueKeyBody,
    QueueKey,
}

impl Item {
    /// Whether `bytes` decode as this item.
    fn decodes(self, bytes: &[u8]) -> Result<(), DecodeError> {
        match self {
            Self::QueuedTask => QueuedTask::from_cbor(bytes).map(drop),
            Self::Sched => Sched::from_cbor(bytes).map(drop),
            Self::TaskBody => TaskBody::from_cbor(bytes).map(drop),
            Self::TaskPlaintext => TaskPlaintext::from_cbor(bytes).map(drop),
            Self::Task => Task::from_cbor(bytes).map(drop),
            Self::TaskResult => TaskResult::from_cbor(bytes).map(drop),
            Self::Claim => Claim::from_cbor(bytes).map(drop),
            Self::QueueKeyBody => QueueKeyBody::from_cbor(bytes).map(drop),
            Self::QueueKey => QueueKey::from_cbor(bytes).map(drop),
        }
    }
}

/// Checks that `bytes`, described as `what`, do not decode as `item`.
fn assert_refused(item: Item, what: &str, bytes: &[u8]) {
    assert!(
        item.decodes(bytes).is_err(),
        "{item:?} refused: {what}: {bytes:02x?}"
    );
}

/// Checks that a `task-body` of `bytes` is refused alone and in a signed task plaintext.
fn assert_body_refused(what: &str, body: &[u8]) {
    assert_refused(Item::TaskBody, what, body);
    assert_refused(Item::TaskPlaintext, what, &signed_cbor(body, &[0x5a; 64]));
}

/// Checks that a `queue-key-body` of `bytes` is refused alone and in a signed queue key object.
fn assert_queue_key_refused(what: &str, body: &[u8]) {
    assert_refused(Item::QueueKeyBody, what, body);
    assert_refused(Item::QueueKey, what, &signed_cbor(body, &[0x5a; 64]));
}

/// Checks that a `queued-task` of `fields` is refused alone and in a claim.
fn assert_queued_refused(ids: &Ids, what: &str, fields: Vec<(i64, Value)>) {
    assert_refused(Item::QueuedTask, what, &encoded(&fields));
    assert_refused(
        Item::Claim,
        &format!("a queued task with {what}"),
        &encoded(&claim_fields(ids, AT_LEAST_ONCE, fields)),
    );
}

/// The fields of `fields`, the one at `key` holding `value`.
fn set(fields: &[(i64, Value)], key: i64, value: Value) -> Vec<(i64, Value)> {
    assert!(fields.iter().any(|(k, _)| *k == key), "no field {key}");
    fields
        .iter()
        .map(|(k, v)| (*k, if *k == key { value.clone() } else { v.clone() }))
        .collect()
}

/// The fields of `fields` without the field at `key`.
fn unset(fields: &[(i64, Value)], key: i64) -> Vec<(i64, Value)> {
    fields.iter().filter(|(k, _)| *k != key).cloned().collect()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_fixtures_decode_as_made() {
    let ids = Ids::new();
    let task_body = encoded(&task_body_fields(&ids, sched_fields(), AT_LEAST_ONCE));
    let queue_key_body = encoded(&queue_key_body_fields(&ids));
    for (item, bytes) in [
        (Item::QueuedTask, encoded(&rich_queued(&ids))),
        (Item::Sched, encoded(&sched_fields())),
        (Item::TaskBody, task_body.clone()),
        (Item::TaskPlaintext, signed(&task_body, SENDER, TASK_LABEL)),
        (Item::Task, encoded(&task_fields(ids.task))),
        (Item::TaskResult, encoded(&result_fields(&ids, DONE))),
        (
            Item::Claim,
            encoded(&claim_fields(&ids, AT_MOST_ONCE, rich_queued(&ids))),
        ),
        (Item::QueueKeyBody, queue_key_body.clone()),
        (
            Item::QueueKey,
            signed(&queue_key_body, SIGNER, QUEUE_KEY_LABEL),
        ),
    ] {
        assert_eq!(item.decodes(&bytes), Ok(()), "{item:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_item_without_a_required_field_is_refused() {
    let ids = Ids::new();
    let queued = queued_fields(&ids, sched_fields());
    for key in 1..=6 {
        assert_queued_refused(&ids, &format!("no field {key}"), unset(&queued, key));
    }
    let body = task_body_fields(&ids, sched_fields(), AT_LEAST_ONCE);
    for key in 1..=8 {
        assert_body_refused(&format!("no field {key}"), &without(&body, key));
    }
    assert_refused(Item::Task, "no request", &encoded(&[]));
    assert_refused(
        Item::Task,
        "no request, a field of a later version",
        &encoded(&crate::fields::unknown()),
    );
    let result = result_fields(&ids, DONE);
    for key in 1..=5 {
        assert_refused(
            Item::TaskResult,
            &format!("no field {key}"),
            &without(&result, key),
        );
    }
    let claim = claim_fields(&ids, AT_LEAST_ONCE, queued.clone());
    for key in 1..=7 {
        assert_refused(
            Item::Claim,
            &format!("no field {key}"),
            &without(&claim, key),
        );
    }
    let queue_key = queue_key_body_fields(&ids);
    for key in 1..=4 {
        assert_queue_key_refused(&format!("no field {key}"), &without(&queue_key, key));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_enc_or_a_queue_key_not_of_32_bytes_is_refused() {
    let ids = Ids::new();
    let queued = queued_fields(&ids, sched_fields());
    let queue_key = queue_key_body_fields(&ids);
    for len in [0, 31, 33] {
        assert_queued_refused(
            &ids,
            &format!("an enc of {len} bytes"),
            set(&queued, 5, bytes(&vec![0xe5; len])),
        );
        assert_queue_key_refused(
            &format!("a key of {len} bytes"),
            &with(&queue_key, 3, bytes(&vec![0x9b; len])),
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_idempotency_key_empty_or_over_32_bytes_is_refused() {
    let ids = Ids::new();
    for (what, key) in [
        ("an empty idempotency key", bytes(&[])),
        ("an idempotency key of 33 bytes", bytes(&[0x6b; 33])),
        ("an idempotency key of text", text("build-1842")),
    ] {
        let sched = set(&sched_fields(), 1, key);
        assert_refused(Item::Sched, what, &encoded(&sched));
        assert_queued_refused(&ids, what, queued_fields(&ids, sched.clone()));
        assert_body_refused(
            what,
            &encoded(&task_body_fields(&ids, sched, AT_LEAST_ONCE)),
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_epoch_generation_or_attempt_of_0_is_refused() {
    let ids = Ids::new();
    assert_queued_refused(
        &ids,
        "epoch 0",
        set(&queued_fields(&ids, vec![]), 4, Value::Uint(0)),
    );
    assert_queue_key_refused(
        "epoch 0",
        &with(&queue_key_body_fields(&ids), 2, Value::Uint(0)),
    );
    let claim = claim_fields(&ids, AT_LEAST_ONCE, queued_fields(&ids, vec![]));
    assert_refused(
        Item::Claim,
        "generation 0",
        &with(&claim, 2, Value::Uint(0)),
    );
    assert_refused(Item::Claim, "attempt 0", &with(&claim, 3, Value::Uint(0)));
    assert_refused(
        Item::TaskResult,
        "generation 0",
        &with(&result_fields(&ids, FAILED), 3, Value::Uint(0)),
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_unknown_queue_mode_or_outcome_is_refused() {
    let ids = Ids::new();
    for code in [0, 3, 255] {
        assert_body_refused(
            &format!("mode {code}"),
            &encoded(&task_body_fields(&ids, vec![], code)),
        );
        assert_refused(
            Item::Claim,
            &format!("mode {code}"),
            &encoded(&claim_fields(&ids, code, queued_fields(&ids, vec![]))),
        );
        assert_refused(
            Item::TaskResult,
            &format!("outcome {code}"),
            &encoded(&result_fields(&ids, code)),
        );
    }
    assert_body_refused(
        "a mode of text",
        &with(
            &task_body_fields(&ids, vec![], AT_LEAST_ONCE),
            6,
            text("at-least-once"),
        ),
    );
    assert_refused(
        Item::TaskResult,
        "an outcome of text",
        &with(&result_fields(&ids, DONE), 4, text("done")),
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_field_of_another_type_or_size_is_refused() {
    let ids = Ids::new();
    let id15 = || bytes(&[0x5a; 15]);
    let id17 = || bytes(&[0x5a; 17]);

    let queued = queued_fields(&ids, sched_fields());
    for (what, key, value) in [
        ("a task of 15 bytes", 1, id15()),
        ("a task of text", 1, text(&ids.task.to_string())),
        ("a queue of 17 bytes", 2, id17()),
        ("a sched that is no map", 3, Value::Array(vec![])),
        ("an epoch of text", 4, text("3")),
        ("an enc of text", 5, text("enc")),
        ("a ct of text", 6, text("ciphertext")),
    ] {
        assert_queued_refused(&ids, what, set(&queued, key, value));
    }

    for (what, key, value) in [
        ("a template that is no name", 4, text("Review PR")),
        ("an empty template", 4, text("")),
        ("a template of bytes", 4, bytes(TEMPLATE.as_bytes())),
        ("a deadline of text", 5, text("1800604800")),
        ("a device of 15 bytes", 6, id15()),
        ("a device of 17 bytes", 6, id17()),
    ] {
        assert_refused(Item::Sched, what, &with(&sched_fields(), key, value));
    }
    assert_refused(Item::Sched, "an array", &Value::Array(vec![]).encode());

    let body = task_body_fields(&ids, sched_fields(), AT_LEAST_ONCE);
    let not_content = Value::Map(vec![(1, text(FROM))]).encode();
    for (what, key, value) in [
        ("a sender of 15 bytes", 1, id15()),
        ("a channel of 17 bytes", 2, id17()),
        ("a queue of 15 bytes", 3, id15()),
        ("a task of 17 bytes", 4, id17()),
        (
            "a sched that is no map",
            5,
            bytes(&encoded(&sched_fields())),
        ),
        ("an enqueued of text", 7, text("now")),
        (
            "content not in a byte string",
            8,
            Value::Map(vec![(1, text(FROM)), (4, Value::Uint(0))]),
        ),
        ("a byte string that is no content", 8, bytes(&not_content)),
        ("a byte string that is no item", 8, bytes(&[0xff])),
    ] {
        assert_body_refused(what, &with(&body, key, value));
    }

    assert_refused(
        Item::Task,
        "a request of bytes",
        &encoded(&[(1, bytes(request(ids.task).as_bytes()))]),
    );

    let result = result_fields(&ids, DONE);
    for (what, key, value) in [
        ("a task of 15 bytes", 1, id15()),
        ("a claim_id of 17 bytes", 2, id17()),
        ("a generation of text", 3, text("4")),
        ("a response of bytes", 5, bytes(response().as_bytes())),
    ] {
        assert_refused(Item::TaskResult, what, &with(&result, key, value));
    }

    let claim = claim_fields(&ids, AT_LEAST_ONCE, queued.clone());
    for (what, key, value) in [
        ("a claim_id of 15 bytes", 1, id15()),
        ("an attempt of text", 3, text("2")),
        ("a lease of text", 4, text("300")),
        ("an uploader of 17 bytes", 6, id17()),
        (
            "a queued task in a byte string",
            7,
            bytes(&encoded(&queued)),
        ),
    ] {
        assert_refused(Item::Claim, what, &with(&claim, key, value));
    }

    let queue_key = queue_key_body_fields(&ids);
    for (what, key, value) in [
        ("a channel of 15 bytes", 1, id15()),
        ("an epoch of text", 2, text("3")),
        ("a signer of 17 bytes", 4, id17()),
    ] {
        assert_queue_key_refused(what, &with(&queue_key, key, value));
    }

    for item in [
        Item::QueuedTask,
        Item::TaskBody,
        Item::Task,
        Item::TaskResult,
        Item::Claim,
        Item::QueueKeyBody,
    ] {
        assert_refused(item, "an array", &Value::Array(vec![]).encode());
        assert_refused(item, "a number", &Value::Uint(1).encode());
        assert_refused(item, "an empty map", &encoded(&[]));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signed_object_that_is_not_a_body_and_a_64_byte_signature_is_refused() {
    let ids = Ids::new();
    for (item, label, seed, body) in [
        (
            Item::TaskPlaintext,
            TASK_LABEL,
            SENDER,
            encoded(&task_body_fields(&ids, vec![], AT_LEAST_ONCE)),
        ),
        (
            Item::QueueKey,
            QUEUE_KEY_LABEL,
            SIGNER,
            encoded(&queue_key_body_fields(&ids)),
        ),
    ] {
        let signature = crate::devices::signing_key(seed).sign(label, &[&body]);
        let (body_item, signature_item) = (bytes(&body), bytes(&signature));
        let array = |items: &[&Value]| Value::Array(items.iter().copied().cloned().collect());
        for (what, bytes) in [
            ("a body alone", array(&[&body_item]).encode()),
            (
                "the signature twice",
                array(&[&body_item, &signature_item, &signature_item]).encode(),
            ),
            (
                "the signature first",
                array(&[&signature_item, &body_item]).encode(),
            ),
            ("a short signature", signed_cbor(&body, &signature[1..])),
            (
                "a long signature",
                signed_cbor(&body, &[&signature[..], &signature[..1]].concat()),
            ),
            ("an encoded body alone", body.clone()),
            (
                "a body of text",
                array(&[&text("task"), &signature_item]).encode(),
            ),
        ] {
            assert_refused(item, what, &bytes);
        }
    }
}

/// An encoding of each item, with fields of later versions, as the tests make them.
fn every_item(ids: &Ids) -> Vec<(Item, Vec<u8>)> {
    let task_body = encoded(&with_unknown(task_body_fields(
        ids,
        with_unknown(sched_fields()),
        AT_MOST_ONCE,
    )));
    let queue_key_body = encoded(&with_unknown(queue_key_body_fields(ids)));
    vec![
        (Item::QueuedTask, encoded(&rich_queued(ids))),
        (Item::Sched, encoded(&with_unknown(sched_fields()))),
        (Item::TaskBody, task_body.clone()),
        (Item::TaskPlaintext, signed(&task_body, SENDER, TASK_LABEL)),
        (Item::Task, encoded(&with_unknown(task_fields(ids.task)))),
        (
            Item::TaskResult,
            encoded(&with_unknown(result_fields(ids, FAILED))),
        ),
        (
            Item::Claim,
            encoded(&with_unknown(claim_fields(
                ids,
                AT_MOST_ONCE,
                rich_queued(ids),
            ))),
        ),
        (Item::QueueKeyBody, queue_key_body.clone()),
        (
            Item::QueueKey,
            signed(&queue_key_body, SIGNER, QUEUE_KEY_LABEL),
        ),
    ]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_truncation_of_an_item_and_bytes_after_it_are_refused() {
    let ids = Ids::new();
    for (item, encoded) in every_item(&ids) {
        assert_eq!(item.decodes(&encoded), Ok(()), "{item:?} as made");
        let not_truncated: Vec<usize> = (0..encoded.len())
            .filter(|&len| item.decodes(&encoded[..len]) != Err(DecodeError::Truncated))
            .collect();
        assert!(
            not_truncated.is_empty(),
            "{item:?}: prefixes not refused as truncated: {not_truncated:?}"
        );
        let longer = [encoded.as_slice(), &[0x00]].concat();
        assert_eq!(
            item.decodes(&longer),
            Err(DecodeError::TrailingBytes(1)),
            "{item:?} with a byte after it"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_item_that_breaks_the_encoding_rules_is_refused() {
    let ids = Ids::new();
    for (item, encoded) in every_item(&ids) {
        let mut broken: Vec<(&str, Vec<u8>)> = vec![
            ("a reserved head", vec![0xfc]),
            ("an indefinite-length map", {
                let mut bytes = encoded.clone();
                bytes[0] = if item_is_signed(item) { 0x9f } else { 0xbf };
                bytes.push(0xff);
                bytes
            }),
        ];
        if !item_is_signed(item) {
            // Map heads of fewer than 24 entries are one byte, and every map's first key is 1: the
            // field 1: 0 before it repeats it.
            let mut repeated = encoded.clone();
            repeated[0] += 1;
            let first = encoded[1];
            repeated.splice(1..1, [first, 0x00]);
            broken.push(("a repeated first key", repeated));
        }
        for (what, bytes) in broken {
            assert_refused(item, what, &bytes);
        }
    }
    let ids = Ids::new();
    // An epoch of 3 as a two-byte head: not the shortest form.
    let mut long_epoch = encoded(&queued_fields(&ids, vec![]));
    // The IDs come before the epoch, so its field is the last 04 03.
    let at = long_epoch
        .windows(2)
        .rposition(|pair| pair == [0x04, 0x03])
        .expect("the epoch field");
    long_epoch.splice(at + 1..at + 2, [0x18, 0x03]);
    assert_refused(
        Item::QueuedTask,
        "an epoch not in its shortest form",
        &long_epoch,
    );
}

/// Whether `item` is a signed object, an array, rather than a map.
fn item_is_signed(item: Item) -> bool {
    matches!(item, Item::TaskPlaintext | Item::QueueKey)
}
