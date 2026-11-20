//! The claim checks that need nothing of the claiming device's state: 1, 3 and, of 2, the
//! signature by the given key, the uploader's. A claimed task passes them only as it was sealed
//! and signed, for its own channel, queue and task, with the record's `sched` and the claim's
//! `mode`.

use skald_crypto::HistoryKey;
use skald_proto::{Cbor, ChannelId, QueueId, QueueMode, TaskBody, TaskId, TaskPlaintext};

use crate::cbor::{Value, bytes};
use crate::devices::{device_id, signing_key};
use crate::fields::signed_cbor;
use crate::fixtures::{
    HISTORY, OTHER_HISTORY, Sealed, assert_refused, body_fields, claim, replaced, sealed,
    sealed_body, sealed_plaintext, verdict,
};
use crate::items::{
    ADDRESSEE, AT_LEAST_ONCE, AT_MOST_ONCE, DEADLINE, ENQUEUED, Ids, QUEUE_KEY_LABEL, SENDER,
    TASK_LABEL, encoded, sched_fields, task_body_fields, with_unknown,
};

/// The verdict of the claim checks on `task`, claimed from its own queue in its own channel in a
/// queue of `mode`, checked with the sender's key.
fn checked(task: &Sealed, mode: QueueMode) -> Result<(), String> {
    verdict(HistoryKey::from_bytes(&HISTORY).queue_key().check_claim(
        &claim(&task.queued, mode),
        task.ids.channel,
        task.ids.queue,
        &signing_key(SENDER).public_key(),
    ))
}

/// A task of `ids` whose body has `body_fields`, signed by [`SENDER`] under `skald task`, its
/// record with every scheduling field.
fn with_body(ids: Ids, body_fields: &[(i64, Value)]) -> Sealed {
    sealed_body(ids, &sched_fields(), body_fields, SENDER, TASK_LABEL)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_claimed_task_that_passes_the_checks_gives_its_body() {
    let task = sealed();

    let body = HistoryKey::from_bytes(&HISTORY)
        .queue_key()
        .check_claim(
            &claim(&task.queued, QueueMode::AtLeastOnce),
            task.ids.channel,
            task.ids.queue,
            &signing_key(SENDER).public_key(),
        )
        .expect("a task sealed and signed as the spec says passes checks 1 to 3");

    assert_eq!(
        body,
        TaskBody::from_cbor(&task.body).expect("the fixture body decodes")
    );
    assert_eq!(body.to_cbor(), task.body, "the body, byte for byte");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_with_fields_of_later_versions_or_without_scheduling_fields_passes_the_checks() {
    let ids = Ids::new();
    let later_body = with_body(ids, &with_unknown(body_fields(&ids)));
    assert_eq!(
        checked(&later_body, QueueMode::AtLeastOnce),
        Ok(()),
        "a body with later fields"
    );

    let ids = Ids::new();
    let later_sched = with_unknown(sched_fields());
    let fields = task_body_fields(&ids, later_sched.clone(), AT_MOST_ONCE);
    let task = sealed_body(ids, &later_sched, &fields, SENDER, TASK_LABEL);
    assert_eq!(
        checked(&task, QueueMode::AtMostOnce),
        Ok(()),
        "a sched with later fields, at-most-once"
    );

    let ids = Ids::new();
    let fields = task_body_fields(&ids, vec![], AT_LEAST_ONCE);
    let task = sealed_body(ids, &[], &fields, SENDER, TASK_LABEL);
    assert_eq!(
        checked(&task, QueueMode::AtLeastOnce),
        Ok(()),
        "an empty sched"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_1_refuses_a_task_that_does_not_open_with_its_epochs_key_and_aad() {
    let task = sealed();
    let signing = signing_key(SENDER).public_key();
    let key = HistoryKey::from_bytes(&HISTORY).queue_key();
    let (channel, queue) = (task.ids.channel, task.ids.queue);
    let claimed = claim(&task.queued, QueueMode::AtLeastOnce);

    assert_refused(
        HistoryKey::from_bytes(&OTHER_HISTORY)
            .queue_key()
            .check_claim(&claimed, channel, queue, &signing),
        "a task checked with another epoch's queue key",
    );
    assert_refused(
        key.check_claim(&claimed, ChannelId::generate(), queue, &signing),
        "a task claimed in another channel",
    );
    assert_refused(
        key.check_claim(&claimed, channel, QueueId::generate(), &signing),
        "a task claimed from another queue",
    );

    let mut changed_ct = claimed.clone();
    changed_ct.task.ct[0] ^= 0x01;
    assert_refused(
        key.check_claim(&changed_ct, channel, queue, &signing),
        "a changed ct",
    );
    let mut other_task = claimed.clone();
    other_task.task.task = TaskId::generate();
    assert_refused(
        key.check_claim(&other_task, channel, queue, &signing),
        "a record that names another task",
    );
    let mut other_epoch = claimed.clone();
    other_epoch.task.epoch += 1;
    assert_refused(
        key.check_claim(&other_epoch, channel, queue, &signing),
        "a record that names another epoch",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_2_refuses_a_task_not_signed_by_the_given_key_under_skald_task() {
    let ids = Ids::new();
    let fields = body_fields(&ids);

    let other_label = sealed_body(ids, &sched_fields(), &fields, SENDER, QUEUE_KEY_LABEL);
    assert_refused(
        checked(&other_label, QueueMode::AtLeastOnce),
        "a task signed under \"skald queue key\"",
    );

    let other_signer = sealed_body(ids, &sched_fields(), &fields, ADDRESSEE, TASK_LABEL);
    assert_refused(
        checked(&other_signer, QueueMode::AtLeastOnce),
        "a task another device signed",
    );

    let task = sealed();
    assert_refused(
        HistoryKey::from_bytes(&HISTORY).queue_key().check_claim(
            &claim(&task.queued, QueueMode::AtLeastOnce),
            task.ids.channel,
            task.ids.queue,
            &signing_key(ADDRESSEE).public_key(),
        ),
        "a task checked with another device's key",
    );

    let other_sender = replaced(&fields, 1, bytes(device_id(ADDRESSEE).as_bytes()));
    assert_refused(
        checked(&with_body(ids, &other_sender), QueueMode::AtLeastOnce),
        "a task whose sender is not the given key's device",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_2_refuses_a_body_changed_after_its_signature() {
    let task = sealed();
    let original =
        TaskPlaintext::from_cbor(&task.plaintext).expect("the fixture plaintext decodes");
    let changed = encoded(&replaced(
        &body_fields(&task.ids),
        7,
        Value::Uint(ENQUEUED + 1),
    ));
    let plaintext = signed_cbor(&changed, original.signature());
    let resealed = sealed_plaintext(task.ids, &sched_fields(), changed, plaintext);

    assert_refused(
        checked(&resealed, QueueMode::AtLeastOnce),
        "a body changed after its signature, sealed again",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_3_refuses_a_body_of_another_channel_queue_or_task() {
    let ids = Ids::new();
    let fields = body_fields(&ids);

    for (what, key, value) in [
        (
            "another channel",
            2,
            bytes(ChannelId::generate().as_bytes()),
        ),
        ("another queue", 3, bytes(QueueId::generate().as_bytes())),
        ("another task", 4, bytes(TaskId::generate().as_bytes())),
    ] {
        let task = with_body(ids, &replaced(&fields, key, value));
        assert_refused(
            checked(&task, QueueMode::AtLeastOnce),
            &format!("a body that names {what}, sealed and signed for this task"),
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_3_refuses_a_sched_that_is_not_the_records_map() {
    let full = sched_fields();
    let without_key: Vec<_> = full.iter().filter(|(k, _)| *k != 1).cloned().collect();
    let later_deadline = replaced(&full, 5, Value::Uint(DEADLINE + 1));
    let other_device = replaced(&full, 6, bytes(device_id(SENDER).as_bytes()));
    let later = with_unknown(full.clone());
    let none = Vec::new();

    for (what, body_sched, record_sched) in [
        (
            "the record without the body's idempotency key",
            &full,
            &without_key,
        ),
        (
            "the body without the record's idempotency key",
            &without_key,
            &full,
        ),
        ("the record with another deadline", &full, &later_deadline),
        (
            "the record with another addressee's device",
            &full,
            &other_device,
        ),
        ("the record with a field of a later version", &full, &later),
        ("the body with a field of a later version", &later, &full),
        ("the record with no scheduling fields", &full, &none),
    ] {
        let ids = Ids::new();
        let fields = task_body_fields(&ids, body_sched.clone(), AT_LEAST_ONCE);
        let task = sealed_body(ids, record_sched, &fields, SENDER, TASK_LABEL);
        assert_refused(checked(&task, QueueMode::AtLeastOnce), what);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn check_3_refuses_a_mode_other_than_the_claims() {
    for (body_mode, claim_mode) in [
        (AT_LEAST_ONCE, QueueMode::AtMostOnce),
        (AT_MOST_ONCE, QueueMode::AtLeastOnce),
    ] {
        let ids = Ids::new();
        let fields = task_body_fields(&ids, sched_fields(), body_mode);
        let task = with_body(ids, &fields);
        assert_refused(
            checked(&task, claim_mode),
            &format!("a body of mode {body_mode} in a claim of {claim_mode:?}"),
        );
    }
}
