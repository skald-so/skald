//! Joining by itself: a device outside a channel's group joins by an external Commit from the
//! latest GroupInfo, after the caller checked its signer and members, and then reads and sends;
//! members get the joiner as the device added. A GroupInfo the caller refuses, or whose signature,
//! tree or channel does not check, leaves no group; an external Commit that removes another
//! device's leaf is refused.

use skald_crypto::{
    ChannelView, GroupChange, Joining, MemberDevice, MlsContent, MlsError, MlsFraming, Processed,
    SealedState,
};
use skald_proto::{ChannelId, DeviceId};

use crate::fixtures::{NOW, device_id, load, new_state, signing_key};
use crate::group_info::{self, EXTERNAL_PUB, GroupInfo, RATCHET_TREE, replace_once, resigned};
use crate::hostile;
use crate::steps::{channel_of, history_key, receive, send, step};

/// Device `seed` as a group lists it.
pub fn member_of(seed: u8) -> MemberDevice {
    MemberDevice {
        device: device_id(seed),
        signing: signing_key(seed).public_key(),
    }
}

/// The GroupInfo device `seed`, loaded from `state`, gives of its current epoch of `channel`, as it
/// uploads it.
pub fn group_info_of(seed: u8, state: &SealedState, channel: ChannelId) -> Vec<u8> {
    let device = load(seed, state);
    let group_info = device.group_info(channel);
    group_info.unwrap_or_else(|err| panic!("device {seed} gives no GroupInfo: {err:?}"))
}

/// Device `seed` tries to join `channel`'s group by itself from `group_info`, `admits` deciding on
/// what it sees; gives what it saw, if it was asked, and the external Commit or why it is refused.
pub fn try_join(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    group_info: &[u8],
    admits: impl FnOnce(&Joining) -> bool,
) -> (Option<Joining>, Result<Vec<u8>, MlsError>) {
    let mut seen = None;
    let joined = step(seed, state, |device| {
        device.join_by_itself(channel, group_info, NOW, |joining| {
            seen = Some(joining.clone());
            admits(joining)
        })
    });
    (seen, joined)
}

/// Device `seed` joins `channel`'s group by itself from `group_info`, admitting what it sees; gives
/// what it saw before the group was saved, and the external Commit.
pub fn joins(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    group_info: &[u8],
) -> (Joining, Vec<u8>) {
    let (seen, joined) = try_join(seed, state, channel, group_info, |_| true);
    let commit = joined.unwrap_or_else(|err| panic!("device {seed} joins by itself: {err:?}"));
    (seen.expect("the caller saw the GroupInfo"), commit)
}

/// What device `seed` sees of `commit`, another device's Commit of `channel`, and whether it enters
/// the new epoch, which it keeps.
pub fn try_process(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    commit: &[u8],
) -> (Option<GroupChange>, Result<Processed, MlsError>) {
    let mut seen = None;
    let entered = step(seed, state, |device| {
        device.process_commit(channel, commit, NOW, |change| {
            seen = Some(change.clone());
            true
        })
    });
    (seen, entered)
}

/// Device `seed` processes `commit` of `channel` and enters its epoch; gives what it saw.
pub fn processed(
    seed: u8,
    state: &mut SealedState,
    channel: ChannelId,
    commit: &[u8],
) -> GroupChange {
    match try_process(seed, state, channel, commit) {
        (Some(change), Ok(Processed::Entered { epoch })) if epoch == change.epoch => change,
        other => panic!("device {seed} does not enter the Commit's epoch: {other:?}"),
    }
}

/// The member devices of `channel`'s group that device `seed` holds, in leaf order.
pub fn members(seed: u8, state: &SealedState, channel: ChannelId) -> Vec<DeviceId> {
    let device = load(seed, state);
    let held = device.members(channel);
    held.unwrap_or_else(|err| panic!("device {seed} holds no group: {err:?}"))
        .iter()
        .map(|member| member.device)
        .collect()
}

/// Fails unless device `seed` is in `epoch` of `channel`'s group, which holds each of the devices
/// `seeds` once and no other, with no Commit of its own pending.
#[track_caller]
pub fn assert_group(seed: u8, state: &SealedState, channel: ChannelId, epoch: u64, seeds: &[u8]) {
    let mut held = members(seed, state, channel);
    held.sort();
    let mut expected: Vec<DeviceId> = seeds.iter().map(|&one| device_id(one)).collect();
    expected.sort();
    let device = load(seed, state);
    assert_eq!(
        (device.epoch(channel).ok(), held),
        (Some(epoch), expected),
        "device {seed}: the epoch and the members, each once"
    );
    let pending = load(seed, state).commit_stored(channel);
    assert!(
        matches!(pending, Err(MlsError::NoCommitPending(_))),
        "device {seed} has a Commit pending: {pending:?}"
    );
}

/// Fails unless device `seed` holds nothing of `channel`'s group: no group, GroupInfo or history
/// key.
#[track_caller]
pub fn assert_no_group(seed: u8, state: &SealedState, channel: ChannelId) {
    let device = load(seed, state);
    assert!(
        device.epoch(channel).is_err()
            && device.members(channel).is_err()
            && device.group_info(channel).is_err(),
        "device {seed} holds a group of {channel}"
    );
    for epoch in 0..8 {
        assert!(
            device.history_key(channel, epoch).is_none(),
            "device {seed} holds a history key of {channel} in epoch {epoch}"
        );
    }
}

/// Fails unless each of `members`, by seed and state, gets the external Commit `commit` of
/// `joiner` as the Commit of `joiner` that adds it, removes `old` and enters `epoch`.
#[track_caller]
pub fn assert_members_get(
    members: &mut [(u8, &mut SealedState)],
    channel: ChannelId,
    commit: &[u8],
    (joiner, old, epoch): (u8, &[u8], u64),
) {
    for (seed, state) in members {
        let change = processed(*seed, state, channel, commit);
        assert_eq!(change.epoch, epoch, "device {seed}: the new epoch");
        assert_eq!(
            change.sender,
            device_id(joiner),
            "device {seed}: the sender"
        );
        assert_eq!(
            change.added,
            [member_of(joiner)],
            "device {seed} gets the joiner as the one device added"
        );
        let removed: Vec<MemberDevice> = old.iter().map(|&one| member_of(one)).collect();
        assert_eq!(change.removed, removed, "device {seed}: the leaves removed");
    }
}

/// Fails unless device `from` sends in `channel` what each of `to` reads, from it.
#[track_caller]
pub fn assert_reads(
    (from, sender): (u8, &mut SealedState),
    to: &mut [(u8, &mut SealedState)],
    channel: ChannelId,
) {
    let text = format!("from device {from}");
    let message = send(from, sender, channel, text.as_bytes());
    for (seed, state) in to {
        assert_eq!(
            receive(*seed, state, channel, &message),
            Some((device_id(from), text.clone().into_bytes())),
            "device {seed} reads what device {from} sends"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_joins_by_itself_from_the_latest_group_info_then_reads_and_sends() {
    let channel = ChannelId::generate();
    let (creator, member, joiner) = (1, 2, 3);
    let mut states = channel_of(channel, creator, &[member]);
    let group_info = group_info_of(creator, &states[0], channel);
    // A new device with no KeyPackage: joining by itself takes none.
    let mut joined = new_state(joiner);

    let (seen, commit) = joins(joiner, &mut joined, channel, &group_info);

    assert_eq!(seen.channel, channel);
    assert_eq!(seen.signer, member_of(creator), "the GroupInfo's signer");
    assert_eq!(
        seen.members,
        [member_of(creator), member_of(member)],
        "every member the GroupInfo lists, in leaf order"
    );
    assert!(
        matches!(
            MlsFraming::read(&commit),
            Some(MlsFraming::Message {
                epoch: 1,
                public: true,
                content: MlsContent::Commit,
                ..
            })
        ),
        "an external Commit of epoch 1 in a PublicMessage"
    );
    let all = [creator, member, joiner];
    assert_group(joiner, &joined, channel, 2, &all);
    let [first, second] = &mut states[..] else {
        panic!("two members");
    };
    assert_members_get(
        &mut [(creator, &mut *first), (member, &mut *second)],
        channel,
        &commit,
        (joiner, &[], 2),
    );
    for (seed, state) in [(creator, &*first), (member, &*second)] {
        assert_group(seed, state, channel, 2, &all);
        assert_eq!(
            members(seed, state, channel),
            members(joiner, &joined, channel),
            "device {seed} and the joiner hold the same leaves"
        );
        assert_eq!(
            history_key(seed, state, channel, 2),
            history_key(joiner, &joined, channel, 2),
            "device {seed} and the joiner derive the same history key"
        );
    }
    assert_reads(
        (joiner, &mut joined),
        &mut [(creator, &mut *first), (member, &mut *second)],
        channel,
    );
    assert_reads((member, second), &mut [(joiner, &mut joined)], channel);
    // The GroupInfo the joiner uploads of the new epoch.
    let uploaded = group_info_of(joiner, &joined, channel);
    let view = ChannelView::observe_joinable(channel, &uploaded).expect("a joinable GroupInfo");
    assert_eq!(
        (view.epoch(), view.signer()),
        (2, member_of(joiner)),
        "the joiner's GroupInfo of the new epoch"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_whose_signer_or_member_the_caller_does_not_admit_leaves_no_group() {
    let channel = ChannelId::generate();
    let (signer, other, not_admitted, joiner) = (11, 12, 13, 14);
    let mut states = channel_of(channel, signer, &[other, not_admitted]);
    let by_admitted = group_info_of(signer, &states[0], channel);
    let by_not_admitted = group_info_of(not_admitted, &states[2], channel);
    // The caller admits every device but one.
    let admits = |joining: &Joining| {
        let mut listed = joining.members.iter().chain([&joining.signer]);
        listed.all(|member| member.device != device_id(not_admitted))
    };
    let mut state = new_state(joiner);

    for (group_info, by) in [(&by_admitted, signer), (&by_not_admitted, not_admitted)] {
        let (seen, joined) = try_join(joiner, &mut state, channel, group_info, admits);

        let seen = seen.expect("the caller saw the GroupInfo");
        assert_eq!(seen.signer, member_of(by), "the GroupInfo's signer");
        assert_eq!(
            seen.members,
            [signer, other, not_admitted].map(member_of),
            "every member the GroupInfo lists, in leaf order"
        );
        assert!(
            matches!(joined, Err(MlsError::Refused)),
            "refused by the caller: {joined:?}"
        );
        assert_no_group(joiner, &state, channel);
    }

    // Once a member removed the device that is not admitted, the latest GroupInfo passes.
    let commit = step(signer, &mut states[0], |device| {
        let commit = (device.remove_devices(channel, &[device_id(not_admitted)], b"", NOW))
            .expect("a Commit of the Remove");
        device.commit_stored(channel).expect("the Commit merges");
        commit
    });
    processed(other, &mut states[1], channel, &commit);
    let latest = group_info_of(signer, &states[0], channel);
    let (_, joined) = try_join(joiner, &mut state, channel, &latest, admits);
    let commit = joined.expect("the device joins from the GroupInfo that passes");
    assert_group(joiner, &state, channel, 3, &[signer, other, joiner]);
    assert_members_get(
        &mut [(other, &mut states[1])],
        channel,
        &commit,
        (joiner, &[], 3),
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_group_info_with_a_bad_signature_or_tree_or_of_another_channel_leaves_no_group() {
    let (channel, elsewhere) = (ChannelId::generate(), ChannelId::generate());
    let (signer, other, joiner, outsider) = (21, 22, 23, 29);
    let mut states = channel_of(channel, signer, &[other]);
    let latest = group_info_of(signer, &states[0], channel);
    // Signed again unchanged, by its signer, it is the same GroupInfo: the tests' encoding holds.
    assert_eq!(resigned(&latest, signer, |_| {}), latest);
    // The other member's leaf lists the outsider instead.
    let altered = |fields: &mut GroupInfo| {
        let tree = fields.extension(RATCHET_TREE);
        let (from, to) = (signing_key(other), signing_key(outsider));
        replace_once(
            tree,
            device_id(other).as_bytes(),
            device_id(outsider).as_bytes(),
        );
        replace_once(tree, &from.public_key(), &to.public_key());
    };
    let mut unsigned = group_info::group_info(&latest);
    altered(&mut unsigned);
    let mut state = new_state(joiner);

    for (group_info, what) in [
        (
            resigned(&latest, outsider, |_| {}),
            "signed again by a key outside the group",
        ),
        (
            resigned(&latest, other, |_| {}),
            "signed again by another member's key than its signer leaf's",
        ),
        (
            resigned(&latest, outsider, |fields| fields.signer = 9),
            "signed for a leaf that does not exist",
        ),
        (unsigned.encode(), "with an altered tree"),
        (
            resigned(&latest, signer, altered),
            "with an altered tree, signed again by its signer",
        ),
        (
            resigned(&latest, signer, |fields| {
                fields.extensions.retain(|(kind, _)| *kind != EXTERNAL_PUB);
            }),
            "without external_pub, signed again by its signer",
        ),
    ] {
        let (_, joined) = try_join(joiner, &mut state, channel, &group_info, |_| true);

        assert!(joined.is_err(), "refused: a GroupInfo {what}");
        assert_no_group(joiner, &state, channel);
    }
    let (_, joined) = try_join(joiner, &mut state, elsewhere, &latest, |_| true);
    assert!(joined.is_err(), "refused: a GroupInfo of another channel");
    assert_no_group(joiner, &state, elsewhere);
    assert_no_group(joiner, &state, channel);

    // The GroupInfo as its signer gave it passes.
    let (_, commit) = joins(joiner, &mut state, channel, &latest);
    assert_group(joiner, &state, channel, 2, &[signer, other, joiner]);
    assert_members_get(
        &mut [(other, &mut states[1])],
        channel,
        &commit,
        (joiner, &[], 2),
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_external_commit_that_removes_another_device_s_leaf_is_refused() {
    let channel = ChannelId::generate();
    let (creator, victim, intruder, joiner) = (31, 32, 33, 34);
    let mut states = channel_of(channel, creator, &[victim]);
    let group_info = group_info_of(creator, &states[0], channel);
    let view = ChannelView::observe(channel, &group_info).expect("the creator's GroupInfo");
    assert_eq!(view.members()[1], member_of(victim), "the victim's leaf, 1");

    let removing = hostile::joins_removing(intruder, &group_info, 1);

    for (at, seed) in [(0, creator), (1, victim)] {
        let (_, entered) = try_process(seed, &mut states[at], channel, &removing);
        assert!(entered.is_err(), "device {seed} refuses it: {entered:?}");
        assert_group(seed, &states[at], channel, 1, &[creator, victim]);
    }
    // The same GroupInfo still takes a device that joins by itself and removes no one else.
    let mut state = new_state(joiner);
    let (_, commit) = joins(joiner, &mut state, channel, &group_info);
    let [first, second] = &mut states[..] else {
        panic!("two members");
    };
    assert_members_get(
        &mut [(creator, first), (victim, second)],
        channel,
        &commit,
        (joiner, &[], 2),
    );
}
