//! An epoch's queue key pair derives from its history key as "Queue key" says.

use skald_crypto::HistoryKey;
use skald_proto::Cbor;

use crate::fixtures::{HISTORY, OTHER_HISTORY, pair, sealed};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_epochs_queue_public_key_derives_from_its_history_key_as_the_spec_says() {
    for history in [HISTORY, OTHER_HISTORY, [0; 16], [0xff; 16]] {
        let (_, pk) = pair(&history);
        assert_eq!(
            HistoryKey::from_bytes(&history).queue_key().public_key(),
            pk,
            "the pk of DeriveKeyPair(HKDF-SHA256(\"\", {history:02x?}, \"skald queue\", 32))"
        );
    }
    assert_ne!(
        HistoryKey::from_bytes(&HISTORY).queue_key().public_key(),
        HistoryKey::from_bytes(&OTHER_HISTORY)
            .queue_key()
            .public_key(),
        "each epoch has its own queue key pair"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_queue_key_pair_opens_a_task_sealed_to_the_spec_derived_public_key() {
    let task = sealed();

    let opened = HistoryKey::from_bytes(&HISTORY)
        .queue_key()
        .open_task(&task.queued, task.ids.channel, task.ids.queue)
        .expect("the sk of the epoch opens a task sealed to its pk");
    assert_eq!(
        opened.to_cbor(),
        task.plaintext,
        "the plaintext, byte for byte"
    );
}
