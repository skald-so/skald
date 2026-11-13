//! Forming a channel's group: `docs/protocol/membership.md`, "Creating a channel". The creator
//! adds devices from their KeyPackages and commits; its Commit merges, and its Welcome is handed
//! out, once the caller says the server stored it.

use skald_proto::ChannelId;

use crate::fixtures::{NOW, admitted, device_id, load, signing_key};
use crate::foreign;
use crate::steps::{
    add, bytes, channel_of, creator, history_key, join, receive, send, step, stored,
    with_key_packages,
};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn devices_added_from_key_packages_reach_epoch_one_with_the_same_members_and_history_key() {
    let channel = ChannelId::generate();
    let seeds = [11, 12, 13];
    let states = channel_of(channel, seeds[0], &seeds[1..]);

    let creator_key = history_key(seeds[0], &states[0], channel, 1);
    assert_ne!(
        creator_key,
        history_key(seeds[0], &states[0], channel, 0),
        "the history key rotates with the epoch"
    );
    for (&seed, state) in seeds.iter().zip(&states) {
        let device = load(seed, state);
        assert_eq!(
            device.epoch(channel).expect("the group"),
            1,
            "device {seed}"
        );
        let members = device.members(channel).expect("the group's members");
        let expected: Vec<_> = seeds
            .iter()
            .map(|&member| (device_id(member), signing_key(member).public_key()))
            .collect();
        let got: Vec<_> = members
            .iter()
            .map(|member| (member.device, member.signing))
            .collect();
        assert_eq!(
            got, expected,
            "device {seed}: the creator, then the added devices, in the order added"
        );
        assert_eq!(
            history_key(seed, state, channel, 1),
            creator_key,
            "device {seed}: the same history key of epoch 1"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_commit_merges_and_its_welcome_is_handed_out_only_once_the_server_stored_it() {
    let channel = ChannelId::generate();
    let (mut joiner, packages) = with_key_packages(15, 1);
    let mut state = creator(14, channel);
    let epoch_zero = history_key(14, &state, channel, 0);

    let commit = add(14, &mut state, channel, &[packages[0].as_slice()]);
    assert!(!commit.is_empty());
    let device = load(14, &state);
    assert_eq!(
        device.epoch(channel).expect("the group"),
        0,
        "the pending Commit has not merged"
    );
    assert_eq!(device.members(channel).expect("the group").len(), 1);
    assert!(device.history_key(channel, 1).is_none(), "no epoch 1 yet");
    assert!(
        load(15, &joiner).join(&commit, NOW, |_| true).is_err(),
        "the Commit is no Welcome"
    );

    // The caller saved the state with the pending Commit before it sent it; the server stored it.
    let welcome = stored(14, &mut state, channel);
    let device = load(14, &state);
    assert_eq!(device.epoch(channel).expect("the group"), 1);
    assert_eq!(device.members(channel).expect("the group").len(), 2);
    assert_eq!(
        history_key(14, &state, channel, 0),
        epoch_zero,
        "the creator keeps the history key of epoch 0"
    );
    assert!(
        step(14, &mut state, |device| device.commit_stored(channel)).is_err(),
        "no Commit is pending once it merged"
    );

    assert_eq!(join(15, &mut joiner, &welcome), channel);
    assert_eq!(load(15, &joiner).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_refused_first_commit_drops_the_group_and_never_hands_out_its_welcome() {
    let channel = ChannelId::generate();
    let (_, packages) = with_key_packages(17, 1);
    let mut state = creator(16, channel);
    add(16, &mut state, channel, &[packages[0].as_slice()]);

    step(16, &mut state, |device| {
        device
            .commit_refused(channel)
            .expect("the refusal is taken");
    });
    let device = load(16, &state);
    assert!(
        device.epoch(channel).is_err(),
        "the server keeps another group of the channel: this one is dropped"
    );
    assert!(device.members(channel).is_err());
    assert!(
        step(16, &mut state, |device| device.commit_stored(channel)).is_err(),
        "the refused Commit's Welcome is never handed out"
    );
    assert!(
        step(16, &mut state, |device| device.encrypt(channel, b"hello")).is_err(),
        "no group to encrypt in"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_creator_whose_first_commit_is_refused_ends_in_the_kept_group() {
    let channel = ChannelId::generate();
    let (first, second, third) = (21, 22, 23);
    let (mut first_state, first_packages) = with_key_packages(first, 1);
    let (mut second_state, second_packages) = with_key_packages(second, 1);
    // The third device's KeyPackages: the server hands each creator one of its own.
    let (mut third_state, third_packages) = with_key_packages(third, 2);

    // Both creators accept the revision with the channel at once and form a group of it.
    for (seed, state) in [(first, &mut first_state), (second, &mut second_state)] {
        step(seed, state, |device| {
            device.create_group(channel, NOW).expect("a new group");
        });
    }
    add(
        first,
        &mut first_state,
        channel,
        &[&second_packages[0], &third_packages[0]],
    );
    add(
        second,
        &mut second_state,
        channel,
        &[&first_packages[0], &third_packages[1]],
    );

    // The server keeps the first creator's Commit and refuses the second's.
    let welcome = stored(first, &mut first_state, channel);
    assert!(
        step(second, &mut second_state, |device| device.join(
            &welcome,
            NOW,
            |_| true
        ))
        .is_err(),
        "a Welcome for a group the device is a member of is refused"
    );
    step(second, &mut second_state, |device| {
        device
            .commit_refused(channel)
            .expect("the refusal is taken");
    });
    assert!(
        step(second, &mut second_state, |device| device
            .commit_stored(channel))
        .is_err(),
        "the refused Commit's Welcome is never handed out"
    );
    assert_eq!(join(second, &mut second_state, &welcome), channel);
    assert_eq!(join(third, &mut third_state, &welcome), channel);

    let kept = load(first, &first_state);
    let kept_members = kept.members(channel).expect("the kept group");
    let kept_key = history_key(first, &first_state, channel, 1);
    for (seed, state) in [(second, &second_state), (third, &third_state)] {
        let device = load(seed, state);
        assert_eq!(device.epoch(channel).expect("the kept group"), 1);
        assert_eq!(
            device.members(channel).expect("the kept group"),
            kept_members,
            "device {seed} is in the kept group"
        );
        assert_eq!(history_key(seed, state, channel, 1), kept_key);
    }

    let message = send(second, &mut second_state, channel, b"in the kept group");
    for (seed, state) in [(first, &mut first_state), (third, &mut third_state)] {
        assert_eq!(
            receive(seed, state, channel, &message),
            Some((device_id(second), b"in the kept group".to_vec())),
            "device {seed} reads the refused creator in the kept group"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn storing_or_refusing_with_no_commit_pending_is_refused_and_leaves_the_group_as_it_was() {
    let channel = ChannelId::generate();
    let mut state = creator(24, channel);
    let key = history_key(24, &state, channel, 0);

    assert!(step(24, &mut state, |device| device.commit_stored(channel)).is_err());
    assert!(step(24, &mut state, |device| device.commit_refused(channel)).is_err());
    let device = load(24, &state);
    assert_eq!(device.epoch(channel).expect("the group stays"), 0);
    assert_eq!(device.members(channel).expect("the group stays").len(), 1);
    assert_eq!(history_key(24, &state, channel, 0), key);

    let other = ChannelId::generate();
    assert!(step(24, &mut state, |device| device.commit_stored(other)).is_err());
    assert!(step(24, &mut state, |device| device.commit_refused(other)).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn adding_to_a_channel_the_device_has_no_group_for_is_refused() {
    let channel = ChannelId::generate();
    let (_, packages) = with_key_packages(26, 1);
    let (mut state, _) = with_key_packages(25, 1);
    assert!(
        step(25, &mut state, |device| device
            .add_devices(channel, &[packages[0].as_slice()], admitted(), NOW)
            .map(bytes))
        .is_err()
    );
    assert!(load(25, &state).epoch(channel).is_err(), "no group made");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_or_truncated_key_package_is_refused_without_a_panic_and_changes_nothing() {
    let channel = ChannelId::generate();
    let (_, packages) = with_key_packages(28, 1);
    let package = packages[0].as_slice();
    let state = creator(27, channel);
    let (_, others) = with_key_packages(29, 1);
    let other_channel = ChannelId::generate();
    let mut other_state = creator(30, other_channel);
    add(30, &mut other_state, other_channel, &[others[0].as_slice()]);
    let welcome = stored(30, &mut other_state, other_channel);

    let mut flipped = package.to_vec();
    let last = flipped.len() - 1;
    flipped[last] ^= 1;
    let mut inputs: Vec<Vec<u8>> = (0..package.len())
        .map(|len| package[..len].to_vec())
        .collect();
    inputs.extend([
        flipped,
        [package, &[0]].concat(),
        b"not a KeyPackage".to_vec(),
        welcome,
    ]);

    let mut device = load(27, &state);
    for input in &inputs {
        let added = device
            .add_devices(channel, &[input.as_slice()], admitted(), NOW)
            .map(bytes);
        assert!(added.is_err(), "refused: {} bytes", input.len());
        assert!(
            device
                .add_devices(channel, &[package, input.as_slice()], admitted(), NOW)
                .map(bytes)
                .is_err(),
            "one bad KeyPackage refuses the Add of all: {} bytes",
            input.len()
        );
    }
    assert_eq!(device.epoch(channel).expect("the group"), 0);
    assert_eq!(device.members(channel).expect("the group").len(), 1);
    assert!(
        device.commit_stored(channel).is_err(),
        "no Commit is pending after refused Adds"
    );

    let mut state = device.seal();
    add(27, &mut state, channel, &[package]);
    stored(27, &mut state, channel);
    assert_eq!(
        load(27, &state).members(channel).expect("the group").len(),
        2
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn adding_a_key_package_that_breaks_the_profile_is_refused() {
    let channel = ChannelId::generate();
    let mut state = creator(31, channel);
    let (own, another) = (device_id(32), device_id(33));
    for (identity, what) in [
        (another.as_bytes().as_slice(), "another device's ID"),
        (&own.as_bytes()[..15], "15 bytes"),
    ] {
        let package = foreign::key_package(32, identity);
        assert!(
            step(31, &mut state, |device| device
                .add_devices(channel, &[package.as_slice()], admitted(), NOW)
                .map(bytes))
            .is_err(),
            "an identity of {what} is refused"
        );
    }
    assert!(step(31, &mut state, |device| device.commit_stored(channel)).is_err());

    // The same client with the device ID of its key keeps to the profile.
    let package = foreign::key_package(32, device_id(32).as_bytes());
    add(31, &mut state, channel, &[package.as_slice()]);
    stored(31, &mut state, channel);
    let members = load(31, &state).members(channel).expect("the group");
    assert_eq!(members[1].device, device_id(32));
}
