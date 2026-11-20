//! A task plaintext sealed to an epoch's `pk` as "Queue key" says, and opened only with that
//! epoch's `sk` and its own `channel`, `queue`, `task` and `epoch` in `aad`: claim check 1.

use skald_crypto::{HistoryKey, seal_task, sign_task};
use skald_proto::{Cbor, ChannelId, QueueId, TaskBody, TaskId, TaskPlaintext};

use crate::devices::signing_key;
use crate::fixtures::{
    HISTORY, OTHER_HISTORY, Sealed, aad, assert_refused, open, pair, record, seal, sealed,
};
use crate::items::{EPOCH, SENDER, sched_fields};

/// The body of `task` signed by [`SENDER`] with the crate.
fn signed_by_the_crate(task: &Sealed) -> TaskPlaintext {
    let body = TaskBody::from_cbor(&task.body).expect("the fixture body decodes");
    sign_task(&body, &signing_key(SENDER)).expect("the crate signs the body")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_sealed_task_is_its_plaintext_sealed_to_the_epochs_public_key_as_the_spec_says() {
    let task = sealed();
    let body = TaskBody::from_cbor(&task.body).expect("the fixture body decodes");
    let plaintext = signed_by_the_crate(&task);
    let (sk, pk) = pair(&HISTORY);

    let queued = seal_task(&plaintext, EPOCH, &pk).expect("the crate seals the plaintext");

    assert_eq!(queued.task, task.ids.task);
    assert_eq!(queued.queue, task.ids.queue);
    assert_eq!(queued.epoch, EPOCH);
    assert_eq!(queued.sched, body.sched, "the record's sched is the body's");
    let opened = open(
        &sk,
        &queued.enc,
        &aad(task.ids.channel, task.ids.queue, task.ids.task, EPOCH),
        &queued.ct,
    )
    .expect("the record opens with the epoch's sk, info and aad as the spec says");
    assert_eq!(
        opened,
        plaintext.to_cbor(),
        "the sealed plaintext, byte for byte"
    );

    let reopened = HistoryKey::from_bytes(&HISTORY)
        .queue_key()
        .open_task(&queued, task.ids.channel, task.ids.queue)
        .expect("the crate opens its own sealed task");
    assert_eq!(reopened, plaintext);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_seal_takes_a_new_ephemeral_key() {
    let task = sealed();
    let plaintext = signed_by_the_crate(&task);
    let (_, pk) = pair(&HISTORY);

    let first = seal_task(&plaintext, EPOCH, &pk).expect("the first seal");
    let second = seal_task(&plaintext, EPOCH, &pk).expect("the second seal");

    assert_ne!(first.enc, second.enc);
    assert_ne!(first.ct, second.ct);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_is_not_sealed_under_epoch_0_or_to_a_key_of_small_order() {
    let task = sealed();
    let plaintext = signed_by_the_crate(&task);
    let (_, pk) = pair(&HISTORY);

    assert_refused(seal_task(&plaintext, 0, &pk), "a seal under epoch 0");
    // X25519 points of small order give an all-zero shared secret: RFC 9180, §7.1.4.
    assert_refused(
        seal_task(&plaintext, EPOCH, &[0; 32]),
        "a seal to a key of small order",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_opens_only_with_its_epochs_private_key() {
    let task = sealed();

    assert_refused(
        HistoryKey::from_bytes(&OTHER_HISTORY)
            .queue_key()
            .open_task(&task.queued, task.ids.channel, task.ids.queue),
        "opening with another epoch's queue key",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_task_opens_only_with_its_own_channel_queue_task_and_epoch_in_aad() {
    let task = sealed();
    let key = HistoryKey::from_bytes(&HISTORY).queue_key();
    let (channel, queue) = (task.ids.channel, task.ids.queue);
    key.open_task(&task.queued, channel, queue)
        .expect("the task opens with its own channel, queue, task and epoch");

    assert_refused(
        key.open_task(&task.queued, ChannelId::generate(), queue),
        "opening with another channel",
    );
    assert_refused(
        key.open_task(&task.queued, channel, QueueId::generate()),
        "opening with another queue",
    );
    let mut other_task = task.queued.clone();
    other_task.task = TaskId::generate();
    assert_refused(
        key.open_task(&other_task, channel, queue),
        "a record that names another task",
    );
    let mut other_epoch = task.queued.clone();
    other_epoch.epoch = EPOCH + 1;
    assert_refused(
        key.open_task(&other_epoch, channel, queue),
        "a record that names another epoch",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_ct_or_enc_does_not_open() {
    let task = sealed();
    let key = HistoryKey::from_bytes(&HISTORY).queue_key();
    let last = task.queued.ct.len() - 1;

    for at in [0, last / 2, last] {
        let mut changed = task.queued.clone();
        changed.ct[at] ^= 0x01;
        assert_refused(
            key.open_task(&changed, task.ids.channel, task.ids.queue),
            &format!("a ct changed at byte {at}"),
        );
    }
    let mut shorter = task.queued.clone();
    shorter.ct.pop();
    assert_refused(
        key.open_task(&shorter, task.ids.channel, task.ids.queue),
        "a ct a byte shorter",
    );
    let mut changed = task.queued.clone();
    changed.enc[0] ^= 0x01;
    assert_refused(
        key.open_task(&changed, task.ids.channel, task.ids.queue),
        "a changed enc",
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_ct_that_opens_to_no_task_plaintext_is_refused() {
    let task = sealed();
    let (_, pk) = pair(&HISTORY);
    let key = HistoryKey::from_bytes(&HISTORY).queue_key();
    let ids = task.ids;
    let aad = aad(ids.channel, ids.queue, ids.task, EPOCH);

    for (what, plaintext) in [
        ("an empty plaintext", Vec::new()),
        ("the body alone", task.body.clone()),
        (
            "the plaintext with a byte after it",
            [task.plaintext.as_slice(), &[0]].concat(),
        ),
    ] {
        let (enc, ct) = seal(&pk, &aad, &plaintext);
        let queued = record(&ids, &sched_fields(), EPOCH, enc, ct);
        assert_refused(key.open_task(&queued, ids.channel, ids.queue), what);
    }
}
