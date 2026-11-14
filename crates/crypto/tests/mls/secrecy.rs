//! Forward secrecy: `docs/protocol/membership.md`, "MLS profile". A device keeps the secrets of
//! the 3 epochs before its current one, for messages stored after a Commit their sender had not
//! processed yet (`docs/protocol/delivery.md`, "Order"), and within an epoch the keys of skipped
//! messages up to 1024 generations ahead, until that epoch's secrets are deleted. A full copy of
//! its sealed state, with its keys, reads no message the device decrypted before the copy was
//! taken, nor one of an epoch the device no longer keeps, nor the Welcome of a regular KeyPackage
//! it joined from. A copy taken before the device's own Commit reads nothing sent after it. History
//! keys stay: the history is outside forward secrecy.

use skald_crypto::{Processed, SealedState};
use skald_proto::ChannelId;

use crate::commits::{empty, process, processes_own_remove, remove, store, try_process};
use crate::fixtures::{NOW, device_id, load, new_state};
use crate::steps::{add, bytes, channel_of, creator, history_key, receive, send, step, stored};

/// The epochs a device keeps before its current one.
const KEPT_EPOCHS: u64 = 3;
/// How many generations ahead of the next one a device decrypts a message within an epoch.
const AHEAD: usize = 1024;

/// Device `committer` commits an empty Commit in `channel`, the server stores it, and device
/// `reader` processes it.
fn next_epoch(
    channel: ChannelId,
    (committer, committer_state): (u8, &mut SealedState),
    (reader, reader_state): (u8, &mut SealedState),
) {
    let commit = empty(committer, committer_state, channel, b"");
    store(committer, committer_state, channel);
    process(reader, reader_state, channel, &commit);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn messages_of_the_three_epochs_before_the_current_one_decrypt_once_and_none_further_back() {
    let channel = ChannelId::generate();
    // Device 91 commits, 92 sends before it processes each Commit, and 93 reads.
    let [mut committer, mut sender, mut reader]: [SealedState; 3] =
        channel_of(channel, 91, &[92, 93]).try_into().unwrap();

    // Each message of epochs 1 to 5 is stored after the Commit that ends its epoch: its sender had
    // not processed that Commit yet.
    let mut sent = Vec::new();
    let mut reader_in_epoch_5 = None;
    for epoch in 1..=5 {
        if epoch == 5 {
            reader_in_epoch_5 = Some(reader.clone());
        }
        let commit = empty(91, &mut committer, channel, b"");
        store(91, &mut committer, channel);
        process(93, &mut reader, channel, &commit);
        let plaintext = format!("sent in epoch {epoch}").into_bytes();
        let message = send(92, &mut sender, channel, &plaintext);
        process(92, &mut sender, channel, &commit);
        sent.push((epoch, message, plaintext));
    }
    assert_eq!(load(93, &reader).epoch(channel).expect("the group"), 6);

    // A copy of the reader in epoch 5 keeps epochs 2 to 4; in epoch 6, epochs 3 to 5.
    let in_epoch_5 = reader_in_epoch_5.expect("the reader in epoch 5");
    for (current, state) in [(5, &in_epoch_5), (6, &reader)] {
        for (epoch, message, plaintext) in &sent {
            if *epoch >= current {
                continue;
            }
            let kept = current - epoch <= KEPT_EPOCHS;
            assert_eq!(
                receive(93, &mut state.clone(), channel, message),
                kept.then(|| (device_id(92), plaintext.clone())),
                "in epoch {current}, a copy decrypts the message of epoch {epoch}: {kept}"
            );
        }
    }

    for (epoch, message, plaintext) in &sent[2..] {
        assert_eq!(
            receive(93, &mut reader, channel, message),
            Some((device_id(92), plaintext.clone())),
            "the reader decrypts the message of epoch {epoch}"
        );
    }
    let copy = reader.clone();
    for (epoch, message, _) in &sent {
        assert_eq!(
            receive(93, &mut copy.clone(), channel, message),
            None,
            "a copy taken after decrypts no message of epoch {epoch}"
        );
    }

    for epoch in 1..=6 {
        assert_eq!(
            history_key(93, &copy, channel, epoch),
            history_key(91, &committer, channel, epoch),
            "the history key of epoch {epoch} stays"
        );
    }
    let message = send(92, &mut sender, channel, b"sent in epoch 6");
    assert_eq!(
        receive(93, &mut reader, channel, &message),
        Some((device_id(92), b"sent in epoch 6".to_vec())),
        "the current epoch reads on"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn skipped_keys_reach_1024_generations_ahead_and_stay_until_their_epochs_secrets_are_deleted() {
    let channel = ChannelId::generate();
    let [mut committer, mut sender, mut reader]: [SealedState; 3] =
        channel_of(channel, 94, &[95, 96]).try_into().unwrap();
    // The sender's messages of epoch 1, generations 0 to 1025.
    let messages: Vec<Vec<u8>> = step(95, &mut sender, |device| {
        (0..=AHEAD + 1)
            .map(|generation| {
                device
                    .encrypt(channel, format!("generation {generation}").as_bytes())
                    .expect("an application message")
            })
            .collect()
    });
    let read = |generation: usize| {
        Some((
            device_id(95),
            format!("generation {generation}").into_bytes(),
        ))
    };

    assert_eq!(
        receive(96, &mut reader.clone(), channel, &messages[AHEAD + 1]),
        None,
        "1025 generations ahead is refused"
    );
    assert_eq!(
        receive(96, &mut reader, channel, &messages[AHEAD]),
        read(AHEAD),
        "1024 generations ahead decrypts"
    );
    assert_eq!(
        receive(96, &mut reader, channel, &messages[AHEAD + 1]),
        read(AHEAD + 1),
        "the next one decrypts in order"
    );
    for generation in [0, AHEAD - 1] {
        assert_eq!(
            receive(96, &mut reader, channel, &messages[generation]),
            read(generation),
            "the skipped generation {generation} decrypts"
        );
    }
    for generation in [0, AHEAD] {
        assert_eq!(
            receive(96, &mut reader.clone(), channel, &messages[generation]),
            None,
            "generation {generation} decrypts once"
        );
    }

    // The skipped keys of epoch 1 stay while the reader keeps epoch 1: up to epoch 4.
    for _ in 0..KEPT_EPOCHS {
        next_epoch(channel, (94, &mut committer), (96, &mut reader));
    }
    assert_eq!(load(96, &reader).epoch(channel).expect("the group"), 4);
    assert_eq!(
        receive(96, &mut reader, channel, &messages[1]),
        read(1),
        "a skipped key of epoch 1 stays in epoch 4"
    );
    next_epoch(channel, (94, &mut committer), (96, &mut reader));
    for generation in [2, AHEAD - 2] {
        assert_eq!(
            receive(96, &mut reader.clone(), channel, &messages[generation]),
            None,
            "in epoch 5, a copy decrypts no skipped generation {generation} of epoch 1"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_copy_taken_after_decrypting_a_message_in_the_current_epoch_does_not_decrypt_it_again() {
    let channel = ChannelId::generate();
    let [mut sender, mut reader]: [SealedState; 2] =
        channel_of(channel, 97, &[98]).try_into().unwrap();
    let first = send(97, &mut sender, channel, b"first");
    let second = send(97, &mut sender, channel, b"second");

    assert_eq!(
        receive(98, &mut reader, channel, &first),
        Some((device_id(97), b"first".to_vec()))
    );
    let copy = reader;
    assert_eq!(
        receive(98, &mut copy.clone(), channel, &first),
        None,
        "the decrypted message's key is deleted"
    );
    assert_eq!(
        receive(98, &mut copy.clone(), channel, &second),
        Some((device_id(97), b"second".to_vec())),
        "the copy decrypts what the device did not"
    );
}

/// Whether a copy of device `seed`'s state, taken after it joined a channel from a Welcome to its
/// KeyPackage, a last-resort one if `last_resort`, and was then removed, opens that Welcome again.
fn opens_its_welcome_again(seed: u8, last_resort: bool) -> bool {
    let channel = ChannelId::generate();
    let mut joiner = new_state(seed);
    let package = step(seed, &mut joiner, |device| {
        let package = if last_resort {
            device.last_resort_key_package(NOW)
        } else {
            device.key_package(NOW)
        };
        bytes(package.expect("a KeyPackage"))
    });
    let mut state = creator(97, channel);
    add(97, &mut state, channel, &[&package]);
    let welcome = stored(97, &mut state, channel);
    let joined = step(seed, &mut joiner, |device| {
        device.join(&welcome, NOW, |_| true)
    });
    assert_eq!(joined.expect("the device joins"), channel);

    // Removed, the device is no member the Welcome could be refused for.
    let commit = remove(97, &mut state, channel, &[seed], b"");
    store(97, &mut state, channel);
    processes_own_remove(seed, &mut joiner, channel, &commit);
    let mut copy = joiner.clone();
    step(seed, &mut copy, |device| {
        device.join(&welcome, NOW, |_| true).is_ok()
    })
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_copy_taken_after_joining_from_a_regular_key_packages_welcome_cannot_open_it_again() {
    assert!(
        !opens_its_welcome_again(99, false),
        "the regular KeyPackage's private keys are deleted"
    );
    assert!(
        opens_its_welcome_again(89, true),
        "the last-resort KeyPackage's keys stay, so its Welcome opens again"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_copy_taken_before_a_devices_own_empty_commit_decrypts_nothing_sent_after_it() {
    let channel = ChannelId::generate();
    let [mut committer, mut other]: [SealedState; 2] =
        channel_of(channel, 97, &[98]).try_into().unwrap();
    let before = send(98, &mut other, channel, b"before");
    // A full copy of device 97's sealed state; its keys are the tests'.
    let mut copy = committer.clone();

    let commit = empty(97, &mut committer, channel, b"");
    store(97, &mut committer, channel);
    process(98, &mut other, channel, &commit);
    let after = [
        (98, send(98, &mut other, channel, b"after")),
        (97, send(97, &mut committer, channel, b"after")),
    ];
    assert_eq!(
        receive(97, &mut committer, channel, &after[0].1),
        Some((device_id(98), b"after".to_vec()))
    );

    assert!(
        !matches!(
            try_process(97, &mut copy, channel, &commit, NOW, true).1,
            Ok(Processed::Entered { .. })
        ),
        "the copy cannot enter the device's own epoch"
    );
    for (sender, message) in &after {
        assert_eq!(
            receive(97, &mut copy.clone(), channel, message),
            None,
            "the copy decrypts nothing device {sender} sent after the Commit"
        );
    }
    assert_eq!(
        receive(97, &mut copy, channel, &before),
        Some((device_id(98), b"before".to_vec())),
        "the copy decrypts what was sent before"
    );
}
