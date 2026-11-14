//! The checks of a Commit a device processes and of a Welcome it joins from:
//! `docs/protocol/membership.md`, "MLS profile" and "KeyPackages", and RFC 9420, §7.2. A Commit
//! whose UpdatePath changes a member's key, that adds a leaf that breaks the profile, or that adds
//! a KeyPackage whose lifetime does not hold at the caller's time or spans more than 60 days is
//! refused and leaves the state as it was. A Welcome is refused if a leaf of its tree has a
//! lifetime that spans more than 60 days, whether or not it still holds; one whose members' leaves
//! only expired joins. A device whose own Commit is pending keeps it and its Welcome through
//! another device's Commit it refuses, and commits nothing else until the server stores or refuses
//! it. The Commits and Welcomes the crate would not make come from a member on mls-rs of the
//! tests' own.

use std::time::Duration;

use mls_rs::{ExtensionList, Group, MlsMessage, client_builder::MlsConfig, time::MlsTime};
use skald_crypto::{MlsDevice, MlsError, Processed, SealedState};
use skald_proto::ChannelId;

use crate::commits::{Seen, assert_in, devices, process, process_at, try_process, try_remove};
use crate::fixtures::{NOW, admitted, device_id, load, new_state, signing_key};
use crate::foreign::{self, SUITE};
use crate::foreign_member::{commit, index_of, other_key, with_foreign_member};
use crate::steps::{
    add, bytes, channel_of, creator, history_key, join, receive, send, step, stored,
    with_key_packages,
};

const HOUR: u64 = 3_600;
pub const DAY: u64 = 24 * HOUR;

/// A KeyPackage, in an `MLSMessage`, of device `seed` on mls-rs, its lifetime from `from` to
/// `from` plus `span`.
pub fn foreign_key_package(seed: u8, from: u64, span: u64) -> MlsMessage {
    let identity = device_id(seed);
    let client = foreign::client(seed, identity.as_bytes(), SUITE, Duration::from_secs(span));
    message(&foreign::key_package_of(&client, from))
}

/// Each of the tests' devices `ours`, with its state in `states`, refuses `commit` and stays in
/// epoch 1 with its 3 members.
fn all_refuse(
    ours: &[u8],
    states: &mut [SealedState],
    channel: ChannelId,
    commit: &[u8],
    what: &str,
) {
    for (at, &seed) in ours.iter().enumerate() {
        assert!(
            try_process(seed, &mut states[at], channel, commit, NOW, true)
                .1
                .is_err(),
            "device {seed} refuses {what}"
        );
        let device = load(seed, &states[at]);
        assert_eq!(device.epoch(channel).expect("the group"), 1, "{what}");
        assert_eq!(
            device.members(channel).expect("the group").len(),
            3,
            "{what}"
        );
        assert!(device.history_key(channel, 2).is_none(), "{what}");
    }
}

/// Each of the tests' devices `ours`, with its state in `states`, processes `commit` and keeps the
/// new epoch; gives what device `ours[0]` saw.
fn all_keep(ours: &[u8], states: &mut [SealedState], channel: ChannelId, commit: &[u8]) -> Seen {
    let mut seen: Vec<Seen> = ours
        .iter()
        .enumerate()
        .map(|(at, &seed)| process(seed, &mut states[at], channel, commit))
        .collect();
    seen.swap_remove(0)
}

/// A KeyPackage in an `MLSMessage` from its bytes.
fn message(package: &[u8]) -> MlsMessage {
    MlsMessage::from_bytes(package).expect("a KeyPackage message")
}

/// The foreign member of `group` commits the Add of each KeyPackage of `refused` in turn, which
/// each of the tests' devices `ours` refuses, then of `kept`, which they keep; gives what device
/// `ours[0]` saw of that.
fn refuse_then_keep<C: MlsConfig>(
    ours: &[u8],
    states: &mut [SealedState],
    channel: ChannelId,
    group: &mut Group<C>,
    refused: [(MlsMessage, &str); 2],
    kept: MlsMessage,
) -> Seen {
    let add = |group: &mut Group<C>, package: MlsMessage| {
        commit(group, |builder| {
            builder
                .add_member(package)
                .expect("mls-rs adds the KeyPackage")
        })
    };
    for (package, what) in refused {
        let added = add(group, package);
        all_refuse(ours, states, channel, &added, what);
    }
    let added = add(group, kept);
    all_keep(ours, states, channel, &added)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_that_adds_and_removes_gives_the_caller_both_before_its_epoch_is_saved() {
    let channel = ChannelId::generate();
    let (mut states, mut group) = with_foreign_member(channel, &[46, 47], 48);
    let (_, packages) = with_key_packages(49, 1);
    let package = MlsMessage::from_bytes(&packages[0]).expect("a KeyPackage message");
    let removed = index_of(&group, 47);
    let revocations = b"the encoded revocations".to_vec();

    let commit = commit(&mut group, |builder| {
        builder
            .add_member(package)
            .expect("mls-rs adds the KeyPackage")
            .remove_member(removed)
            .expect("mls-rs removes the member")
            .authenticated_data(revocations.clone())
    });
    assert_eq!(
        process(46, &mut states[0], channel, &commit),
        Seen {
            epoch: 2,
            sender: device_id(48),
            added: devices(&[49]),
            removed: devices(&[47]),
            authenticated_data: revocations,
        }
    );
    // The added device takes the leftmost blank leaf, the removed one's.
    assert_in(46, &states[0], channel, 2, &[46, 49, 48], "the Commit kept");
    assert!(matches!(
        try_process(47, &mut states[1], channel, &commit, NOW, true).1,
        Ok(Processed::Removed)
    ));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_whose_update_path_changes_a_members_key_is_refused_and_leaves_the_state_as_it_was() {
    let channel = ChannelId::generate();
    let ours = [50, 51];
    let (mut states, mut group) = with_foreign_member(channel, &ours, 52);

    // The member's leaf keeps its device ID, 52's, and takes key 53: mls-rs's basic
    // `valid_successor` lets it through.
    let (secret, identity) = other_key(52, 53);
    let changed = commit(&mut group, |builder| {
        builder.set_new_signing_identity(secret, identity)
    });
    all_refuse(
        &ours,
        &mut states,
        channel,
        &changed,
        "a member's key changed",
    );

    let kept = commit(&mut group, |builder| builder);
    let seen = all_keep(&ours, &mut states, channel, &kept);
    assert_eq!(
        seen.sender,
        device_id(52),
        "the member's empty Commit is kept"
    );
    let member = load(50, &states[0]).members(channel).expect("the group")[2];
    assert_eq!(
        (member.device, member.signing),
        (device_id(52), signing_key(52).public_key()),
        "the member keeps its key"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_that_adds_a_leaf_that_breaks_the_profile_is_refused_and_leaves_the_state_as_it_was() {
    let channel = ChannelId::generate();
    let ours = [54, 55];
    let (mut states, mut group) = with_foreign_member(channel, &ours, 56);

    let another = device_id(58);
    let refused = [
        (
            foreign::key_package(57, another.as_bytes()),
            "the device ID of another key",
        ),
        (
            foreign::key_package(57, b"not a device ID"),
            "an identity that is not a device ID",
        ),
    ]
    .map(|(package, what)| (message(&package), what));
    let kept = message(&foreign::key_package(57, device_id(57).as_bytes()));
    let seen = refuse_then_keep(&ours, &mut states, channel, &mut group, refused, kept);
    assert_eq!(seen.added, devices(&[57]), "a leaf to the profile is added");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_that_adds_a_key_package_spanning_more_than_sixty_days_is_refused_and_leaves_the_state_as_it_was()
 {
    let channel = ChannelId::generate();
    let ours = [59, 60];
    let (mut states, mut group) = with_foreign_member(channel, &ours, 61);

    let refused = [
        (60 * DAY + 1, "a lifetime of 60 days and a second"),
        (365 * DAY, "a lifetime of 365 days"),
    ]
    .map(|(span, what)| (foreign_key_package(62, NOW, span), what));
    let kept = foreign_key_package(62, NOW, 60 * DAY);
    let seen = refuse_then_keep(&ours, &mut states, channel, &mut group, refused, kept);
    assert_eq!(seen.added, devices(&[62]), "a lifetime of 60 days is added");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_commit_that_adds_a_key_package_not_valid_at_the_callers_time_is_refused_and_leaves_the_state_as_it_was()
 {
    // Device 64's KeyPackage, made at the tests' time, holds from an hour before it to 60 days
    // after that; device 65's, made two hours later, from an hour after the tests' time.
    for (joiner, made, committed, processed, what) in [
        (64, NOW, NOW, NOW + 61 * DAY, "expired at the caller's time"),
        (
            65,
            NOW + 2 * HOUR,
            NOW + 3 * HOUR,
            NOW,
            "not yet valid at the caller's time",
        ),
    ] {
        let channel = ChannelId::generate();
        let mut states = channel_of(channel, 63, &[66]);
        let mut joining = new_state(joiner);
        let package = step(joiner, &mut joining, |device: &mut MlsDevice| {
            bytes(device.key_package(made).expect("a KeyPackage"))
        });
        let commit = step(63, &mut states[0], |device| {
            bytes(
                device
                    .add_devices(channel, &[&package], admitted(), committed)
                    .expect("a KeyPackage valid when it is added"),
            )
        });
        step(63, &mut states[0], |device| {
            device.commit_stored(channel).expect("the Commit merges");
        });

        assert!(
            try_process(66, &mut states[1], channel, &commit, processed, true)
                .1
                .is_err(),
            "refused: a KeyPackage {what}"
        );
        assert_in(66, &states[1], channel, 1, &[63, 66], what);
        let seen = process_at(66, &mut states[1], channel, &commit, committed);
        assert_eq!(seen.added, devices(&[joiner]), "kept at a time it holds");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_welcome_with_a_leaf_spanning_more_than_sixty_days_is_refused_and_one_with_leaves_only_expired_joins()
 {
    // The leaf of the group's creator on mls-rs: its lifetime from `from` for `span`.
    for (from, span, what) in [
        (NOW, 60 * DAY + 1, "60 days and a second, which still holds"),
        (NOW, 365 * DAY, "365 days, which still holds"),
        (NOW - 400 * DAY, 365 * DAY, "365 days, which expired"),
    ] {
        let channel = ChannelId::generate();
        let (mut joiner, packages) = with_key_packages(68, 1);
        let identity = device_id(67);
        let client = foreign::client(67, identity.as_bytes(), SUITE, Duration::from_secs(span));
        let mut group = client
            .create_group_with_id(
                channel.as_bytes().to_vec(),
                ExtensionList::new(),
                ExtensionList::new(),
                Some(MlsTime::from(from)),
            )
            .expect("a group of mls-rs");
        let output = group
            .commit_builder()
            .commit_time(MlsTime::from(NOW))
            .add_member(MlsMessage::from_bytes(&packages[0]).expect("a KeyPackage message"))
            .expect("mls-rs adds the KeyPackage")
            .build()
            .expect("a Commit of mls-rs");
        let welcome = output.welcome_messages[0]
            .to_bytes()
            .expect("the Welcome's bytes");

        assert!(
            step(68, &mut joiner, |device| device
                .join(&welcome, NOW, |_| true))
            .is_err(),
            "refused: a leaf whose lifetime spans {what}"
        );
        assert!(
            load(68, &joiner).epoch(channel).is_err(),
            "{what}: no group"
        );
    }

    // The creator's leaf, made at the tests' time, spans 60 days and expired a month ago.
    let channel = ChannelId::generate();
    let now = NOW + 90 * DAY;
    let mut state = creator(69, channel);
    let mut joining = new_state(70);
    let package = step(70, &mut joining, |device| {
        bytes(device.key_package(now).expect("a KeyPackage"))
    });
    step(69, &mut state, |device| {
        device
            .add_devices(channel, &[&package], admitted(), now)
            .expect("a Commit of the Add");
    });
    let welcome = step(69, &mut state, |device| {
        device
            .commit_stored(channel)
            .expect("the Commit merges")
            .expect("a Welcome")
    });
    assert_eq!(
        step(70, &mut joining, |device| device.join(
            &welcome,
            now,
            |_| true
        ))
        .expect("a Welcome whose leaves only expired joins"),
        channel
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_pending_commit_and_its_welcome_outlast_refused_commits_and_block_another_commit() {
    let channel = ChannelId::generate();
    let ours = [87, 88];
    let (mut states, mut group) = with_foreign_member(channel, &ours, 89);
    let (mut joiner, packages) = with_key_packages(90, 1);
    let pending = add(87, &mut states[0], channel, &[&packages[0]]);

    let empty = commit(&mut group, |builder| builder);
    let (seen, outcome) = try_process(87, &mut states[0], channel, &empty, NOW, false);
    assert_eq!(
        seen.expect("the caller sees the Commit").sender,
        device_id(89)
    );
    assert!(
        matches!(outcome, Err(MlsError::Refused)),
        "the caller refuses the epoch: {outcome:?}"
    );
    let spanning = commit(&mut group, |builder| {
        builder
            .add_member(foreign_key_package(91, NOW, 365 * DAY))
            .expect("mls-rs adds the KeyPackage")
    });
    let outcome = try_process(87, &mut states[0], channel, &spanning, NOW, true).1;
    assert!(
        matches!(outcome, Err(MlsError::Lifetime)),
        "a leaf spanning 365 days: {outcome:?}"
    );
    let outcome = try_remove(87, &mut states[0], channel, &[88], b"");
    assert!(
        outcome.is_err(),
        "a Remove is refused: a Commit is pending already"
    );
    let outcome = step(87, &mut states[0], |device| {
        device.commit_empty(channel, b"", NOW)
    });
    assert!(
        outcome.is_err(),
        "an empty Commit is refused: a Commit is pending already"
    );

    let welcome = stored(87, &mut states[0], channel);
    assert_in(
        87,
        &states[0],
        channel,
        2,
        &[87, 88, 89, 90],
        "the pending Commit merged",
    );
    assert_eq!(
        process(88, &mut states[1], channel, &pending).added,
        devices(&[90])
    );
    assert_eq!(
        history_key(87, &states[0], channel, 2),
        history_key(88, &states[1], channel, 2)
    );
    assert_eq!(join(90, &mut joiner, &welcome), channel);
    let message = send(87, &mut states[0], channel, b"in epoch 2");
    for (seed, state) in [(88, &mut states[1]), (90, &mut joiner)] {
        assert_eq!(
            receive(seed, state, channel, &message),
            Some((device_id(87), b"in epoch 2".to_vec())),
            "device {seed}"
        );
    }
}
