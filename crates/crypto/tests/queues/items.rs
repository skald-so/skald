//! Each item of `docs/protocol/queues.md` decodes into its spec fields and encodes back byte for
//! byte, fields of later versions included; a signed object keeps its body's bytes and signature.

use std::fmt::Debug;

use skald_proto::{
    Cbor, Claim, Outcome, QueueKey, QueueKeyBody, QueueMode, QueuedTask, Sched, SignedBody, Task,
    TaskBody, TaskPlaintext, TaskResult, Unknown,
};

use crate::cbor::{Value, bytes};
use crate::devices::device_id;
use crate::fields::assert_signed_round_trip;
use crate::fixtures::*;

/// The keys of `unknown`, sorted.
fn keys(unknown: &Unknown) -> Vec<i128> {
    let mut keys: Vec<i128> = unknown.keys().collect();
    keys.sort_unstable();
    keys
}

/// The keys of the fields of later versions in [`crate::fields::unknown`].
const LATER: [i128; 2] = [-3, 24];

/// Decodes `encoded` as a `T` and checks that it encodes back byte for byte.
fn round_trip<T: Cbor + Debug>(what: &str, encoded: &[u8]) -> T {
    let decoded =
        T::from_cbor(encoded).unwrap_or_else(|error| panic!("{what} does not decode: {error:?}"));
    assert_eq!(decoded.to_cbor(), encoded, "{what}, byte for byte");
    decoded
}

/// Checks that `sched` holds every field of [`sched_fields`].
fn assert_full_sched(sched: &Sched) {
    assert_eq!(sched.idempotency_key.as_deref(), Some(IDEMPOTENCY_KEY));
    assert_eq!(
        sched.template.as_ref().map(ToString::to_string).as_deref(),
        Some(TEMPLATE)
    );
    assert_eq!(sched.deadline, Some(DEADLINE));
    assert_eq!(sched.device, Some(device_id(ADDRESSEE)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_queued_task_decodes_into_its_spec_fields_and_round_trips_with_unknown_fields() {
    let ids = Ids::new();
    let queued: QueuedTask = round_trip("the queued task", &encoded(&rich_queued(&ids)));

    assert_eq!(queued.task, ids.task);
    assert_eq!(queued.queue, ids.queue);
    assert_eq!(queued.epoch, EPOCH);
    assert_eq!(queued.enc, ENC);
    assert_eq!(queued.ct, CT);
    assert_full_sched(&queued.sched);
    assert_eq!(
        keys(&queued.sched.unknown),
        LATER,
        "the sched's unknown fields"
    );
    assert_eq!(
        keys(&queued.unknown),
        LATER,
        "the queued task's unknown fields"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_queued_task_without_scheduling_fields_round_trips() {
    let ids = Ids::new();
    let queued: QueuedTask = round_trip("the queued task", &encoded(&queued_fields(&ids, vec![])));
    let sched = &queued.sched;
    assert_eq!(sched.idempotency_key, None);
    assert!(sched.template.is_none());
    assert_eq!(sched.deadline, None);
    assert_eq!(sched.device, None);
    assert!(sched.unknown.is_empty());
    assert!(queued.unknown.is_empty());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn scheduling_fields_round_trip_alone_each_optional() {
    let sched: Sched = round_trip("the sched", &encoded(&with_unknown(sched_fields())));
    assert_full_sched(&sched);
    assert_eq!(keys(&sched.unknown), LATER);

    for field in sched_fields() {
        let key = field.0;
        let sched: Sched = round_trip(&format!("a sched of field {key} alone"), &encoded(&[field]));
        assert_eq!(sched.idempotency_key.is_some(), key == 1, "field {key}");
        assert_eq!(sched.template.is_some(), key == 4, "field {key}");
        assert_eq!(sched.deadline.is_some(), key == 5, "field {key}");
        assert_eq!(sched.device.is_some(), key == 6, "field {key}");
    }
    let empty: Sched = round_trip("an empty sched", &encoded(&[]));
    assert!(empty.unknown.is_empty());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn idempotency_keys_of_1_to_32_bytes_decode() {
    for len in [1, 32] {
        let key = vec![0x6b; len];
        let sched: Sched = round_trip(
            &format!("an idempotency key of {len} bytes"),
            &encoded(&[(1, bytes(&key))]),
        );
        assert_eq!(sched.idempotency_key.as_deref(), Some(key.as_slice()));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_body_decodes_into_its_spec_fields_and_round_trips_with_unknown_fields() {
    let ids = Ids::new();
    let sched = with_unknown(sched_fields());
    let fields = with_unknown(task_body_fields(&ids, sched.clone(), AT_MOST_ONCE));
    let body: TaskBody = round_trip("the task body", &encoded(&fields));

    assert_eq!(body.sender, device_id(SENDER));
    assert_eq!(body.channel, ids.channel);
    assert_eq!(body.queue, ids.queue);
    assert_eq!(body.task, ids.task);
    assert_eq!(body.sched, Sched::from_cbor(&encoded(&sched)).unwrap());
    assert_full_sched(&body.sched);
    assert_eq!(body.mode, QueueMode::AtMostOnce);
    assert_eq!(body.enqueued, ENQUEUED);
    assert_eq!(body.content.from.to_string(), FROM);
    assert_eq!(
        body.content.to.as_ref().map(ToString::to_string).as_deref(),
        Some(TO)
    );
    assert!(body.content.counter.is_none(), "a task has no counter");
    assert_eq!(body.content.to_cbor(), content(&ids), "the content");
    assert_eq!(keys(&body.unknown), LATER);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_plaintext_keeps_its_body_bytes_and_signature_under_the_task_label() {
    assert_eq!(<TaskBody as SignedBody>::LABEL, TASK_LABEL);
    let ids = Ids::new();
    let body = encoded(&with_unknown(task_body_fields(
        &ids,
        with_unknown(sched_fields()),
        AT_LEAST_ONCE,
    )));
    let decoded = TaskBody::from_cbor(&body).unwrap();
    assert_signed_round_trip::<TaskBody>(&body, &decoded);

    let signed = signed(&body, SENDER, TASK_LABEL);
    let plaintext: TaskPlaintext = round_trip("the task plaintext", &signed);
    assert_eq!(plaintext.body(), &decoded);
    assert_eq!(plaintext.body_bytes(), body);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_queue_mode_decodes_to_its_own_value() {
    let ids = Ids::new();
    for (code, mode) in [
        (AT_LEAST_ONCE, QueueMode::AtLeastOnce),
        (AT_MOST_ONCE, QueueMode::AtMostOnce),
    ] {
        let body: TaskBody = round_trip(
            &format!("a task body of mode {code}"),
            &encoded(&task_body_fields(&ids, vec![], code)),
        );
        assert_eq!(body.mode, mode, "task body, mode {code}");
        let claim: Claim = round_trip(
            &format!("a claim of mode {code}"),
            &encoded(&claim_fields(&ids, code, queued_fields(&ids, vec![]))),
        );
        assert_eq!(claim.mode, mode, "claim, mode {code}");
    }
    assert_ne!(QueueMode::AtLeastOnce, QueueMode::AtMostOnce);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_body_holds_its_request_and_round_trips_with_unknown_fields() {
    let task = skald_proto::TaskId::generate();
    let decoded: Task = round_trip("the task", &encoded(&with_unknown(task_fields(task))));
    assert_eq!(decoded.request, request(task));
    assert_eq!(keys(&decoded.unknown), LATER);

    let plain: Task = round_trip("a task of a request alone", &encoded(&task_fields(task)));
    assert!(plain.unknown.is_empty());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_result_decodes_into_its_spec_fields_and_round_trips_with_each_outcome() {
    let ids = Ids::new();
    for (code, outcome) in [(DONE, Outcome::Done), (FAILED, Outcome::Failed)] {
        let result: TaskResult = round_trip(
            &format!("a result of outcome {code}"),
            &encoded(&with_unknown(result_fields(&ids, code))),
        );
        assert_eq!(result.task, ids.task);
        assert_eq!(result.claim_id, ids.claim);
        assert_eq!(result.generation, GENERATION);
        assert_eq!(result.outcome, outcome, "outcome {code}");
        assert_eq!(result.response, response());
        assert_eq!(keys(&result.unknown), LATER);
    }
    assert_ne!(Outcome::Done, Outcome::Failed);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_claim_decodes_into_its_spec_fields_and_round_trips_with_its_queued_task() {
    let ids = Ids::new();
    let queued = rich_queued(&ids);
    let fields = with_unknown(claim_fields(&ids, AT_LEAST_ONCE, queued.clone()));
    let claim: Claim = round_trip("the claim", &encoded(&fields));

    assert_eq!(claim.claim_id, ids.claim);
    assert_eq!(claim.generation, GENERATION);
    assert_eq!(claim.attempt, ATTEMPT);
    assert_eq!(claim.lease, LEASE);
    assert_eq!(claim.mode, QueueMode::AtLeastOnce);
    assert_eq!(claim.uploader, device_id(SENDER));
    assert_eq!(
        claim.task,
        QueuedTask::from_cbor(&encoded(&queued)).unwrap()
    );
    assert_eq!(claim.task.task, ids.task);
    assert_full_sched(&claim.task.sched);
    assert_eq!(claim.task.to_cbor(), encoded(&queued), "the queued task");
    assert_eq!(keys(&claim.unknown), LATER);

    let lease_0: Claim = round_trip(
        "a claim of lease 0",
        &crate::fields::with(
            &claim_fields(&ids, AT_LEAST_ONCE, queued),
            4,
            Value::Uint(0),
        ),
    );
    assert_eq!(lease_0.lease, 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_queue_key_object_keeps_its_body_bytes_and_signature_under_the_queue_key_label() {
    assert_eq!(<QueueKeyBody as SignedBody>::LABEL, QUEUE_KEY_LABEL);
    let ids = Ids::new();
    let body = encoded(&with_unknown(queue_key_body_fields(&ids)));
    let decoded: QueueKeyBody = round_trip("the queue key body", &body);
    assert_eq!(decoded.channel, ids.channel);
    assert_eq!(decoded.epoch, EPOCH);
    assert_eq!(decoded.key, KEY);
    assert_eq!(decoded.signer, device_id(SIGNER));
    assert_eq!(keys(&decoded.unknown), LATER);
    assert_signed_round_trip::<QueueKeyBody>(&body, &decoded);

    let object: QueueKey = round_trip(
        "the queue key object",
        &signed(&body, SIGNER, QUEUE_KEY_LABEL),
    );
    assert_eq!(object.body(), &decoded);
    assert_eq!(object.body_bytes(), body);

    let plain: QueueKeyBody = round_trip(
        "a body without unknown fields",
        &encoded(&queue_key_body_fields(&ids)),
    );
    assert!(plain.unknown.is_empty());
}
