//! Removing a device: `docs/protocol/membership.md`, "Group changes", and `mls-prototypes.md`,
//! "Removed device". One Commit removes one device or several, such as every device of an
//! account. A device that processes its own Remove drops the group: it neither encrypts nor
//! decrypts there, keeps the group's history keys, and a Welcome can add it again. A full copy of
//! a removed device's sealed state, with its keys, reads nothing sent after the Remove, and members
//! refuse what it sends.

use skald_proto::ChannelId;

use crate::commits::{
    assert_in, devices, empty, process, processes_own_remove, remove, store, try_process,
};
use crate::fixtures::{NOW, device_id, load};
use crate::steps::{add, bytes, channel_of, history_key, join, receive, send, step, stored};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_removed_device_decrypts_nothing_sent_after_its_removal_even_from_a_copy_of_its_state() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 30, &[31, 32]);
    // A full copy of device 32's sealed state, taken before the Remove; its keys are the tests'.
    let copy = states[2].clone();

    let commit = remove(30, &mut states[0], channel, &[32], b"");
    store(30, &mut states[0], channel);
    process(31, &mut states[1], channel, &commit);
    let sent = [
        (
            30,
            send(30, &mut states[0], channel, b"from 30 after the Remove"),
        ),
        (
            31,
            send(31, &mut states[1], channel, b"from 31 after the Remove"),
        ),
    ];
    assert_eq!(
        receive(31, &mut states[1], channel, &sent[0].1),
        Some((device_id(30), b"from 30 after the Remove".to_vec())),
        "the members read it"
    );

    let mut processed = copy.clone();
    processes_own_remove(32, &mut processed, channel, &commit);
    for (what, state) in [
        ("the copy", &copy),
        ("the copy after it processed the Remove", &processed),
    ] {
        for (sender, message) in &sent {
            assert_eq!(
                receive(32, &mut state.clone(), channel, message),
                None,
                "{what} decrypts nothing device {sender} sent after the Remove"
            );
        }
        assert!(
            load(32, state).history_key(channel, 2).is_none(),
            "{what} holds no history key of the epoch after the Remove"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn members_refuse_what_a_removed_device_sends_from_a_copy_of_its_state() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 33, &[34, 35]);
    let mut copy = states[2].clone();

    let commit = remove(33, &mut states[0], channel, &[35], b"");
    store(33, &mut states[0], channel);
    process(34, &mut states[1], channel, &commit);

    let message = send(35, &mut copy, channel, b"from the copy");
    let stale = empty(35, &mut copy, channel, b"");
    for (at, seed) in [(0, 33), (1, 34)] {
        assert_eq!(
            receive(seed, &mut states[at], channel, &message),
            None,
            "device {seed} refuses the copy's message"
        );
        assert!(
            try_process(seed, &mut states[at], channel, &stale, NOW, true)
                .1
                .is_err(),
            "device {seed} refuses the copy's Commit"
        );
        assert_in(
            seed,
            &states[at],
            channel,
            2,
            &[33, 34],
            "after the copy's Commit",
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_that_processes_its_own_remove_drops_the_group_and_keeps_its_history_keys() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 36, &[37, 38]);
    let commit = empty(36, &mut states[0], channel, b"");
    store(36, &mut states[0], channel);
    for (at, seed) in [(1, 37), (2, 38)] {
        process(seed, &mut states[at], channel, &commit);
    }

    let commit = remove(36, &mut states[0], channel, &[38], b"");
    store(36, &mut states[0], channel);
    process(37, &mut states[1], channel, &commit);
    processes_own_remove(38, &mut states[2], channel, &commit);

    let device = load(38, &states[2]);
    assert!(device.epoch(channel).is_err(), "the group is dropped");
    assert!(device.members(channel).is_err(), "the group is dropped");
    for epoch in [1, 2] {
        assert_eq!(
            history_key(38, &states[2], channel, epoch),
            history_key(36, &states[0], channel, epoch),
            "the history key of epoch {epoch} is kept"
        );
    }
    assert!(device.history_key(channel, 3).is_none());
    assert!(
        step(38, &mut states[2], |device| device
            .encrypt(channel, b"after the Remove")
            .map(bytes))
        .is_err(),
        "the removed device encrypts nothing"
    );
    let message = send(36, &mut states[0], channel, b"after the Remove");
    assert_eq!(
        receive(38, &mut states[2], channel, &message),
        None,
        "the removed device decrypts nothing"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_removed_device_is_added_again_from_a_welcome() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 39, &[40, 41]);
    let commit = remove(39, &mut states[0], channel, &[41], b"");
    store(39, &mut states[0], channel);
    process(40, &mut states[1], channel, &commit);
    processes_own_remove(41, &mut states[2], channel, &commit);

    let package = step(41, &mut states[2], |device| {
        bytes(device.key_package(NOW).expect("a KeyPackage"))
    });
    let commit = add(39, &mut states[0], channel, &[&package]);
    let welcome = stored(39, &mut states[0], channel);
    assert_eq!(
        process(40, &mut states[1], channel, &commit).added,
        devices(&[41])
    );
    assert_eq!(join(41, &mut states[2], &welcome), channel);

    for (at, seed) in [(0, 39), (1, 40), (2, 41)] {
        assert_in(seed, &states[at], channel, 3, &[39, 40, 41], "added again");
    }
    assert_eq!(
        history_key(41, &states[2], channel, 1),
        history_key(39, &states[0], channel, 1),
        "the history key of its first epoch is kept"
    );
    assert!(load(41, &states[2]).history_key(channel, 2).is_none());
    assert_eq!(
        history_key(41, &states[2], channel, 3),
        history_key(39, &states[0], channel, 3)
    );
    let message = send(39, &mut states[0], channel, b"welcome back");
    assert_eq!(
        receive(41, &mut states[2], channel, &message),
        Some((device_id(39), b"welcome back".to_vec()))
    );
    let message = send(41, &mut states[2], channel, b"back");
    for (at, seed) in [(0, 39), (1, 40)] {
        assert_eq!(
            receive(seed, &mut states[at], channel, &message),
            Some((device_id(41), b"back".to_vec()))
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn one_commit_removes_every_device_of_an_account() {
    let channel = ChannelId::generate();
    // Devices 44 and 45 are every device of one account.
    let mut states = channel_of(channel, 42, &[43, 44, 45]);

    let commit = remove(42, &mut states[0], channel, &[44, 45], b"");
    store(42, &mut states[0], channel);
    assert_in(
        42,
        &states[0],
        channel,
        2,
        &[42, 43],
        "one Commit, one epoch",
    );
    let seen = process(43, &mut states[1], channel, &commit);
    assert_eq!((seen.epoch, seen.removed), (2, devices(&[44, 45])));
    assert_in(
        43,
        &states[1],
        channel,
        2,
        &[42, 43],
        "one Commit, one epoch",
    );
    for (at, seed) in [(2, 44), (3, 45)] {
        processes_own_remove(seed, &mut states[at], channel, &commit);
    }

    let message = send(42, &mut states[0], channel, b"after the account's removal");
    assert_eq!(
        receive(43, &mut states[1], channel, &message),
        Some((device_id(42), b"after the account's removal".to_vec()))
    );
    for (at, seed) in [(2, 44), (3, 45)] {
        assert_eq!(receive(seed, &mut states[at], channel, &message), None);
    }
}
