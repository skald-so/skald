//! The device's MLS state reaches the caller only sealed, under a key the crate makes and the
//! caller keeps in key storage, each item under its own nonce and associated data; a state that is
//! not the device's own, as it sealed it, is refused.

use std::collections::BTreeSet;

use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::ChannelId;

use crate::fixtures::{
    after, holds, items, load, new_state, signing_key, state_key, state_of, state_with_groups,
};

/// Whether device `seed`, with its own keys, refuses `state`.
fn refused(seed: u8, state: Vec<(Vec<u8>, Vec<u8>)>) -> bool {
    MlsDevice::load(&signing_key(seed), &state_key(seed), &state_of(state)).is_err()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_state_loads_with_the_keys_restored_from_key_storage() {
    let signing = SigningKey::generate();
    let state_key = StateKey::generate();
    let channel = ChannelId::generate();
    let mut device = MlsDevice::new(&signing, &state_key);
    device.create_group(channel).expect("a new group");
    let key = *device.history_key(channel, 0).expect("epoch 0").to_bytes();
    let state = device.seal();

    let stored_signing = SigningKey::from_bytes(&signing.to_bytes());
    let stored_state_key = StateKey::from_bytes(&state_key.to_bytes());
    let device = MlsDevice::load(&stored_signing, &stored_state_key, &state)
        .expect("the state loads with the stored keys");
    assert_eq!(
        *device.history_key(channel, 0).expect("epoch 0").to_bytes(),
        key
    );
    assert_eq!(device.epoch(channel).expect("the group"), 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_sealed_state_holds_no_private_key_or_history_key_in_the_clear() {
    let channels = [ChannelId::generate(), ChannelId::generate()];
    let state = state_with_groups(10, &channels);
    let device = load(10, &state);
    let history_keys: Vec<[u8; 16]> = channels
        .iter()
        .map(|&channel| *device.history_key(channel, 0).expect("epoch 0").to_bytes())
        .collect();

    assert!(
        !holds(&state, signing_key(10).to_bytes().as_ref()),
        "the device's signing key"
    );
    assert!(
        !holds(&state, state_key(10).to_bytes().as_ref()),
        "the state key"
    );
    for key in &history_keys {
        assert!(!holds(&state, key), "a history key");
    }
    // The state does hold what it should, sealed: the groups load from it.
    assert_eq!(device.members(channels[1]).expect("the group").len(), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_seal_takes_a_fresh_nonce_for_every_item() {
    let state = state_with_groups(11, &[ChannelId::generate()]);
    let device = load(11, &state);
    let (first, second) = (items(&device.seal()), items(&device.seal()));
    assert!(!first.is_empty());
    let names = |items: &[(Vec<u8>, Vec<u8>)]| -> Vec<Vec<u8>> {
        items.iter().map(|(name, _)| name.clone()).collect()
    };
    assert_eq!(names(&first), names(&second), "the same items");
    for ((name, once), (_, again)) in first.iter().zip(&second) {
        assert_ne!(once, again, "item {name:?} sealed twice alike");
    }
    for sealed in [first, second] {
        assert!(!refused(11, sealed), "each seal loads");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_group_is_sealed_as_an_item_of_its_own() {
    let (first, second) = (ChannelId::generate(), ChannelId::generate());
    let one = state_with_groups(12, &[first]);
    let two = after(12, &one, |device| {
        device.create_group(second).expect("a second group");
    });
    let names =
        |state| -> BTreeSet<Vec<u8>> { items(state).into_iter().map(|(name, _)| name).collect() };
    let (one, two) = (names(&one), names(&two));
    assert!(
        two.len() > one.len() && one.is_subset(&two),
        "a new group adds an item: {one:?}, then {two:?}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_altered_state_is_refused() {
    let state = items(&state_with_groups(13, &[ChannelId::generate()]));
    assert!(!refused(13, state.clone()));
    for at in 0..state.len() {
        let len = state[at].1.len();
        for byte in [0, len / 2, len - 1] {
            let mut altered = state.clone();
            altered[at].1[byte] ^= 0x01;
            assert!(refused(13, altered), "item {at}, byte {byte} flipped");
        }
        let mut longer = state.clone();
        longer[at].1.push(0);
        assert!(refused(13, longer), "item {at} with a byte appended");

        let mut renamed = state.clone();
        renamed[at].0.push(0);
        assert!(refused(13, renamed), "item {at} under another name");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_truncated_or_malformed_state_is_refused_without_a_panic() {
    let state = items(&state_with_groups(14, &[ChannelId::generate()]));
    for at in 0..state.len() {
        let len = state[at].1.len();
        let step = (len / 64).max(1);
        for cut in (0..len).filter(|cut| *cut < 48 || cut % step == 0) {
            let mut truncated = state.clone();
            truncated[at].1.truncate(cut);
            assert!(refused(14, truncated), "item {at} cut to {cut} bytes");
        }
    }
    for (name, sealed) in [
        (Vec::new(), Vec::new()),
        (Vec::new(), vec![0; 64]),
        (b"group".to_vec(), vec![0xff; 200]),
        (state[0].0.clone(), Vec::new()),
        (state[0].0.clone(), vec![0; 12]),
    ] {
        let mut malformed = state.clone();
        malformed.retain(|(other, _)| *other != name);
        malformed.push((name.clone(), sealed));
        assert!(refused(14, malformed), "item {name:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_state_sealed_under_another_key_is_refused() {
    let state = state_with_groups(15, &[ChannelId::generate()]);
    for other in [StateKey::generate(), state_key(16)] {
        assert!(MlsDevice::load(&signing_key(15), &other, &state).is_err());
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_state_made_by_another_device_is_refused() {
    // Both devices seal under the same state key, so only the device each item names tells them
    // apart.
    let shared = state_key(17);
    let seal = |seed: u8| {
        let mut device = MlsDevice::new(&signing_key(seed), &shared);
        device.key_package().expect("a KeyPackage");
        device.create_group(ChannelId::generate()).expect("a group");
        items(&device.seal())
    };
    let (theirs, ours) = (seal(18), seal(19));
    let ours_loads = |items: Vec<(Vec<u8>, Vec<u8>)>| {
        MlsDevice::load(&signing_key(19), &shared, &state_of(items)).is_ok()
    };
    assert!(ours_loads(ours.clone()));
    assert!(!ours_loads(theirs.clone()), "another device's whole state");
    for item in theirs {
        let mut mixed = ours.clone();
        mixed.retain(|(name, _)| *name != item.0);
        mixed.push(item.clone());
        assert!(!ours_loads(mixed), "another device's item {:?}", item.0);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn one_item_in_place_of_another_is_refused() {
    // Two groups and two KeyPackages: one group's sealed state in place of the other's, and any
    // other item in place of any other.
    let state = after(
        20,
        &state_with_groups(20, &[ChannelId::generate(), ChannelId::generate()]),
        |device| {
            device.key_package().expect("a second KeyPackage");
        },
    );
    let state = items(&state);
    assert!(state.len() >= 2, "{} items", state.len());
    for from in 0..state.len() {
        for to in (0..state.len()).filter(|to| *to != from) {
            let mut swapped = state.clone();
            swapped[to].1 = state[from].1.clone();
            assert!(refused(20, swapped), "item {from} in place of item {to}");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_new_device_reloads_with_no_group() {
    let channel = ChannelId::generate();
    let device = load(21, &new_state(21));
    assert!(device.epoch(channel).is_err());
    let state = after(21, &new_state(21), |_| {});
    assert!(!refused(21, items(&state)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_state_keys_differ() {
    let keys: BTreeSet<[u8; 32]> = (0..8).map(|_| *StateKey::generate().to_bytes()).collect();
    assert_eq!(keys.len(), 8);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn state_and_history_keys_never_show_their_bytes() {
    assert_eq!(format!("{:?}", state_key(22)), "StateKey(..)");
    assert_eq!(format!("{:#?}", StateKey::generate()), "StateKey(..)");

    let channel = ChannelId::generate();
    let device = load(22, &state_with_groups(22, &[channel]));
    let key = device.history_key(channel, 0).expect("epoch 0");
    assert_eq!(format!("{key:?}"), "HistoryKey(..)");
    assert_eq!(format!("{key:#?}"), "HistoryKey(..)");
}
