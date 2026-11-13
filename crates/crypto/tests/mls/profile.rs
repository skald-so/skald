//! KeyPackages and a channel's group to the MLS profile: `docs/protocol/membership.md`, "MLS
//! profile", "Channels" and "History key".

use skald_crypto::MlsDevice;
use skald_proto::ChannelId;

use crate::fixtures::{
    after, device_id, load, new_state, signing_key, state_key, state_with_groups,
};
use crate::wire::{self, BASIC_CREDENTIAL, FROM_KEY_PACKAGE, PROFILE_SUITE};

/// Checks that `bytes` is a KeyPackage of device `seed` to the profile; gives it read.
fn assert_profile(seed: u8, bytes: &[u8]) -> wire::KeyPackage {
    let package = wire::key_package(bytes);
    let public = signing_key(seed).public_key();
    assert_eq!(package.cipher_suite, PROFILE_SUITE, "the ciphersuite");
    assert_eq!(
        package.leaf.credential_type, BASIC_CREDENTIAL,
        "a basic credential"
    );
    assert_eq!(
        package.leaf.identity,
        device_id(seed).as_bytes(),
        "the identity is the device ID"
    );
    assert_eq!(
        package.leaf.signature_key, public,
        "the signature key is the device's signing key"
    );
    assert_eq!(package.leaf.source, FROM_KEY_PACKAGE);
    let suites: Vec<u16> = package
        .leaf
        .cipher_suites
        .iter()
        .copied()
        .filter(|&suite| !wire::is_grease(suite))
        .collect();
    assert_eq!(suites, [PROFILE_SUITE], "the only ciphersuite enabled");
    assert!(
        wire::verifies_with_label(
            &public,
            "LeafNodeTBS",
            &package.leaf.to_be_signed,
            &package.leaf.signature
        ),
        "the device's signing key signs the leaf"
    );
    assert!(
        wire::verifies_with_label(
            &public,
            "KeyPackageTBS",
            &package.to_be_signed,
            &package.signature
        ),
        "the device's signing key signs the KeyPackage"
    );
    package
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_key_package_keeps_to_the_mls_profile() {
    let mut device = load(1, &new_state(1));
    assert_eq!(device.id(), device_id(1));
    let package = device.key_package().expect("a KeyPackage");
    assert_profile(1, package.as_ref());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_makes_distinct_key_packages_and_keeps_their_keys_across_reloads() {
    let mut state = new_state(2);
    let mut packages = Vec::new();
    let mut sizes = vec![sealed_size(&state)];
    for _ in 0..3 {
        state = after(2, &state, |device| {
            let package = device.key_package().expect("a KeyPackage");
            packages.push(assert_profile(2, package.as_ref()));
        });
        sizes.push(sealed_size(&state));
    }
    for (at, package) in packages.iter().enumerate() {
        for other in &packages[at + 1..] {
            assert_ne!(package.init_key, other.init_key, "a fresh init key");
            assert_ne!(
                package.leaf.encryption_key, other.leaf.encryption_key,
                "a fresh leaf encryption key"
            );
        }
    }
    assert!(
        sizes.windows(2).all(|pair| pair[0] < pair[1]),
        "each KeyPackage's private keys join the sealed state and stay after a reload: {sizes:?}"
    );
}

fn sealed_size(state: &skald_crypto::SealedState) -> usize {
    state
        .items()
        .map(|(name, sealed)| name.len() + sealed.len())
        .sum()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_new_group_is_at_epoch_zero_with_the_device_as_its_one_member() {
    let channel = ChannelId::generate();
    let device = load(3, &state_with_groups(3, &[channel]));
    assert_eq!(device.epoch(channel).expect("the group's epoch"), 0);
    let members = device.members(channel).expect("the group's members");
    assert_eq!(members.len(), 1, "{members:?}");
    assert_eq!(members[0].device, device_id(3));
    assert_eq!(members[0].signing, signing_key(3).public_key());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_new_group_holds_the_history_key_of_epoch_zero_across_reloads() {
    let (first, second) = (ChannelId::generate(), ChannelId::generate());
    let one_group = state_with_groups(4, &[first]);
    let key = *load(4, &one_group)
        .history_key(first, 0)
        .expect("the history key of epoch 0")
        .to_bytes();

    let two_groups = after(4, &one_group, |device| {
        device.create_group(second).expect("a second group");
    });
    let device = load(4, &two_groups);
    assert_eq!(
        *device.history_key(first, 0).expect("still held").to_bytes(),
        key,
        "the same key after reloads and another group"
    );
    let other = *device
        .history_key(second, 0)
        .expect("the second group's history key")
        .to_bytes();
    assert_ne!(other, key, "each group has a history key of its own");
    assert_ne!(key, [0; 16]);
    assert!(device.history_key(first, 1).is_none(), "no epoch 1 yet");
    assert_eq!(device.epoch(first).expect("the first group"), 0);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_holds_nothing_of_a_channel_it_has_no_group_for() {
    let held = ChannelId::generate();
    let other = ChannelId::generate();
    let device = load(5, &state_with_groups(5, &[held]));
    assert!(device.epoch(other).is_err());
    assert!(device.members(other).is_err());
    assert!(device.history_key(other, 0).is_none());

    let new = MlsDevice::new(&signing_key(5), &state_key(5));
    assert!(new.epoch(held).is_err(), "a new device has no group");
    assert!(new.history_key(held, 0).is_none());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn creating_a_group_the_device_holds_is_refused_and_leaves_the_group_as_it_was() {
    let channel = ChannelId::generate();
    let state = state_with_groups(6, &[channel]);
    let key = *load(6, &state)
        .history_key(channel, 0)
        .expect("the history key of epoch 0")
        .to_bytes();

    let mut device = load(6, &state);
    assert!(device.create_group(channel).is_err());
    let device = load(6, &device.seal());
    assert_eq!(
        *device
            .history_key(channel, 0)
            .expect("still held")
            .to_bytes(),
        key,
        "the group was not replaced"
    );
    assert_eq!(device.epoch(channel).expect("the group"), 0);
    assert_eq!(device.members(channel).expect("the group").len(), 1);
}
