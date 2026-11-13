//! Joining from a Welcome: `docs/protocol/membership.md`, "Group changes" and "MLS profile". The
//! caller checks the Welcome's signer and members before the joined group is saved; a refused
//! Welcome leaves no group.

use skald_crypto::{MemberDevice, SealedState};
use skald_proto::ChannelId;

use crate::fixtures::{NOW, device_id, load, signing_key};
use crate::foreign;
use crate::framing::{self, WELCOME};
use crate::steps::{
    add, bytes, channel_of, creator, history_key, join, receive, send, step, stored,
    with_key_packages,
};
use crate::wire::PROFILE_SUITE;

fn member(seed: u8) -> MemberDevice {
    MemberDevice {
        device: device_id(seed),
        signing: signing_key(seed).public_key(),
    }
}

/// `channel`'s group of device `creator_seed`, which added `joiners` and the server stored; gives
/// the creator's state, each joiner's state with its KeyPackage's keys, and the Welcome.
fn welcomed(
    channel: ChannelId,
    creator_seed: u8,
    joiners: &[u8],
) -> (SealedState, Vec<SealedState>, Vec<u8>) {
    let (states, packages): (Vec<_>, Vec<_>) = joiners
        .iter()
        .map(|&seed| {
            let (state, mut packages) = with_key_packages(seed, 1);
            (state, packages.remove(0))
        })
        .unzip();
    let mut state = creator(creator_seed, channel);
    let packages: Vec<&[u8]> = packages.iter().map(Vec::as_slice).collect();
    add(creator_seed, &mut state, channel, &packages);
    let welcome = stored(creator_seed, &mut state, channel);
    (state, states, welcome)
}

/// Whether device `seed` holds anything of `channel`'s group in `state`, after a reload.
fn holds_group(seed: u8, state: &SealedState, channel: ChannelId) -> bool {
    let device = load(seed, state);
    assert_eq!(
        device.epoch(channel).is_ok(),
        device.members(channel).is_ok()
    );
    device.epoch(channel).is_ok() || device.history_key(channel, 1).is_some()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_welcomes_signer_and_members_reach_the_caller_before_the_group_is_saved() {
    let channel = ChannelId::generate();
    let (_, mut joiners, welcome) = welcomed(channel, 41, &[42, 43]);
    assert_eq!(framing::welcome_suite(&welcome), PROFILE_SUITE);

    let mut seen = None;
    let joined = step(42, &mut joiners[0], |device| {
        device.join(&welcome, NOW, |joining| {
            seen = Some((
                joining.channel,
                joining.epoch,
                joining.signer,
                joining.members.clone(),
            ));
            true
        })
    });
    assert_eq!(joined.expect("the checks pass: it joins"), channel);
    let (seen_channel, epoch, signer, members) = seen.expect("the caller saw the Welcome");
    assert_eq!(seen_channel, channel);
    assert_eq!(epoch, 1);
    assert_eq!(signer, member(41), "the creator signed the Welcome");
    assert_eq!(
        members,
        [member(41), member(42), member(43)],
        "every member, with its device ID and signature key, in leaf order"
    );
    let device = load(42, &joiners[0]);
    assert_eq!(device.members(channel).expect("the joined group"), members);
    assert_eq!(device.epoch(channel).expect("the joined group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_the_caller_refuses_leaves_no_group() {
    let channel = ChannelId::generate();
    let (mut creator_state, mut joiners, welcome) = welcomed(channel, 44, &[45]);

    let mut asked = false;
    let joined = step(45, &mut joiners[0], |device| {
        let joined = device.join(&welcome, NOW, |_| {
            asked = true;
            false
        });
        assert!(device.epoch(channel).is_err(), "no group after the refusal");
        joined
    });
    assert!(asked, "the caller was asked");
    assert!(joined.is_err(), "the caller refused the Welcome");
    assert!(
        !holds_group(45, &joiners[0], channel),
        "a refused Welcome leaves no group"
    );

    let message = send(44, &mut creator_state, channel, b"after the refusal");
    assert_eq!(receive(45, &mut joiners[0], channel, &message), None);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_for_a_group_the_device_is_a_member_of_is_refused() {
    let channel = ChannelId::generate();
    let mut states = channel_of(channel, 46, &[47]);
    let key = history_key(47, &states[1], channel, 1);
    let members = load(47, &states[1]).members(channel).expect("the group");

    // Another device forms a group of the same channel and adds the member with a new KeyPackage.
    let package = step(47, &mut states[1], |device| {
        bytes(device.key_package(NOW).expect("a KeyPackage"))
    });
    let mut other = creator(48, channel);
    add(48, &mut other, channel, &[package.as_slice()]);
    let other_welcome = stored(48, &mut other, channel);

    let joined = step(47, &mut states[1], |device| {
        device.join(&other_welcome, NOW, |_| true)
    });
    assert!(joined.is_err(), "refused, not a silent replacement");
    let device = load(47, &states[1]);
    assert_eq!(device.epoch(channel).expect("the group stays"), 1);
    assert_eq!(device.members(channel).expect("the group stays"), members);
    assert_eq!(history_key(47, &states[1], channel, 1), key);

    let message = send(46, &mut states[0], channel, b"still in the group");
    assert_eq!(
        receive(47, &mut states[1], channel, &message),
        Some((device_id(46), b"still in the group".to_vec()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_with_a_leaf_that_breaks_the_profile_is_refused_and_leaves_no_group() {
    let channel = ChannelId::generate();
    let (signer, joiner, other) = (51, 52, 53);
    let honest = device_id(signer);
    let another = device_id(54);
    let short = &honest.as_bytes()[..15];
    let long = [honest.as_bytes().as_slice(), &[0]].concat();
    let breaking_member = foreign::key_package(other, another.as_bytes());

    for (identity, extra, what) in [
        (
            another.as_bytes().as_slice(),
            None,
            "the signer's identity is another device's ID",
        ),
        (short, None, "the signer's identity is 15 bytes"),
        (&long, None, "the signer's identity is 17 bytes"),
        (
            honest.as_bytes(),
            Some(breaking_member.as_slice()),
            "another member's identity is another device's ID",
        ),
    ] {
        let (mut state, packages) = with_key_packages(joiner, 1);
        let mut added: Vec<&[u8]> = vec![&packages[0]];
        added.extend(extra);
        let welcome = foreign::welcome(signer, identity, channel, &added);
        let joined = step(joiner, &mut state, |device| {
            device.join(&welcome, NOW, |_| true)
        });
        assert!(joined.is_err(), "refused: {what}");
        assert!(!holds_group(joiner, &state, channel), "no group: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_that_keeps_to_the_profile_joins_whichever_client_made_it() {
    let channel = ChannelId::generate();
    let (signer, joiner, other) = (55, 56, 57);
    let (mut state, packages) = with_key_packages(joiner, 1);
    let third = foreign::key_package(other, device_id(other).as_bytes());
    let welcome = foreign::welcome(
        signer,
        device_id(signer).as_bytes(),
        channel,
        &[&packages[0], &third],
    );

    let mut seen = None;
    let joined = step(joiner, &mut state, |device| {
        device.join(&welcome, NOW, |joining| {
            seen = Some((joining.signer, joining.members.clone()));
            true
        })
    });
    assert_eq!(joined.expect("it joins"), channel);
    let (seen_signer, members) = seen.expect("the caller saw the Welcome");
    assert_eq!(seen_signer, member(signer));
    assert_eq!(members, [member(signer), member(joiner), member(other)]);
    assert_eq!(load(joiner, &state).epoch(channel).expect("the group"), 1);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_that_was_not_added_cannot_join_from_the_welcome() {
    let channel = ChannelId::generate();
    let (_, _, welcome) = welcomed(channel, 58, &[59]);
    let (mut outsider, _) = with_key_packages(60, 1);
    let mut asked = false;
    let joined = step(60, &mut outsider, |device| {
        device.join(&welcome, NOW, |_| {
            asked = true;
            true
        })
    });
    assert!(joined.is_err());
    assert!(
        !asked,
        "nothing to check: the Welcome is not for this device"
    );
    assert!(!holds_group(60, &outsider, channel));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_malformed_or_truncated_welcome_is_refused_without_a_panic_and_leaves_no_group() {
    let channel = ChannelId::generate();
    let (_, joiners, welcome) = welcomed(channel, 61, &[62]);
    assert_eq!(
        u16::from_be_bytes([welcome[2], welcome[3]]),
        WELCOME,
        "a Welcome in an MLSMessage"
    );

    let mut inputs: Vec<Vec<u8>> = (0..welcome.len())
        .map(|len| welcome[..len].to_vec())
        .collect();
    for at in [welcome.len() - 1, welcome.len() / 2, 4] {
        let mut flipped = welcome.clone();
        flipped[at] ^= 1;
        inputs.push(flipped);
    }
    inputs.extend([
        [welcome.as_slice(), &[0]].concat(),
        b"not a Welcome".to_vec(),
    ]);

    let mut device = load(62, &joiners[0]);
    for input in &inputs {
        assert!(
            device.join(input, NOW, |_| true).is_err(),
            "refused: {} bytes",
            input.len()
        );
        assert!(device.epoch(channel).is_err(), "no group");
    }
    let mut state = device.seal();
    assert!(!holds_group(62, &state, channel));
    assert_eq!(
        join(62, &mut state, &welcome),
        channel,
        "the refusals changed nothing: the intact Welcome joins"
    );
}
