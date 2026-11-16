//! Rejoining after missing a Commit: the device fails on the next Commit and stays in its epoch,
//! then joins by itself from the latest GroupInfo with an external Commit that also removes its old
//! leaf. Members get it as the Remove of its old leaf and its Add; the device ends with one leaf,
//! and all read each other.

use skald_crypto::SealedState;
use skald_proto::ChannelId;

use crate::fixtures::{NOW, device_id};
use crate::joins::{
    assert_group, assert_members_get, assert_reads, group_info_of, joins, member_of, members,
    processed, try_process,
};
use crate::steps::{channel_of, history_key, step};

/// Device `seed` commits an empty Commit in `channel`, which the server stores; gives the Commit.
fn empty_commit(seed: u8, state: &mut SealedState, channel: ChannelId) -> Vec<u8> {
    step(seed, state, |device| {
        let commit = (device.commit_empty(channel, b"", NOW)).expect("an empty Commit");
        let welcome = device.commit_stored(channel).expect("the Commit merges");
        assert!(welcome.is_none(), "an empty Commit has no Welcome");
        commit
    })
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_device_that_missed_a_commit_rejoins_with_one_leaf_and_members_see_its_old_leaf_removed() {
    let channel = ChannelId::generate();
    let (committer, missing, other) = (41, 42, 43);
    let all = [committer, missing, other];
    let mut states = channel_of(channel, committer, &[missing, other]);
    let [first, second, third] = &mut states[..] else {
        panic!("three members");
    };
    // Every device enters epoch 2; the device then misses the Commit of epoch 3.
    let commit = empty_commit(committer, first, channel);
    processed(missing, second, channel, &commit);
    processed(other, third, channel, &commit);
    let missed = empty_commit(committer, first, channel);
    processed(other, third, channel, &missed);
    let next = empty_commit(committer, first, channel);
    processed(other, third, channel, &next);

    let (_, failed) = try_process(missing, second, channel, &next);

    assert!(
        failed.is_err(),
        "the Commit after the missed one fails: {failed:?}"
    );
    assert_group(missing, second, channel, 2, &all);
    let latest = group_info_of(committer, first, channel);

    let (seen, rejoin) = joins(missing, second, channel, &latest);

    assert!(
        seen.members.contains(&member_of(missing)),
        "the GroupInfo lists the device's old leaf"
    );
    assert_group(missing, second, channel, 5, &all);
    let held = members(missing, second, channel);
    assert_eq!(
        held.iter()
            .filter(|&&device| device == device_id(missing))
            .count(),
        1,
        "the device has one leaf: {held:?}"
    );
    assert_members_get(
        &mut [(committer, &mut *first), (other, &mut *third)],
        channel,
        &rejoin,
        (missing, &[missing], 5),
    );
    for (seed, state) in [(committer, &*first), (other, &*third)] {
        assert_group(seed, state, channel, 5, &all);
        assert_eq!(
            members(seed, state, channel),
            held,
            "device {seed} and the rejoined device hold the same leaves"
        );
        assert_eq!(
            history_key(seed, state, channel, 5),
            history_key(missing, second, channel, 5),
            "device {seed} and the rejoined device derive the same history key"
        );
    }
    assert_reads(
        (missing, &mut *second),
        &mut [(committer, &mut *first), (other, &mut *third)],
        channel,
    );
    assert_reads(
        (committer, first),
        &mut [(missing, &mut *second), (other, &mut *third)],
        channel,
    );
}
