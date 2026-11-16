//! Rotation as an own Commit: Ada and Bob noticing an aged epoch at once give one new epoch, and a
//! lost `stored` or a CLI killed after sending its Commit leaves one new epoch after the next run.

use crate::aged::{
    DAY, ahead, assert_rotated_once, assert_talk_in, epochs, next_epochs, talk_channels,
    with_period,
};
use crate::fixtures::kill_once;
use crate::front::Answers;
use crate::kit::{RECV, assert_members, group, received};
use crate::played::Took;
use crate::talk::{Talk, commit_sent};
use crate::world::{now, run};

/// What Ada and Bob say in the new epoch.
const ADA_SAYS: &str = "one rotation, not two";
const BOB_SAYS: &str = "agreed, a single new epoch";

/// A talk whose space's period a revision of Ada's device sets to a day, which Ada's `skald recv`
/// accepts with the epochs just made; then Ada's machine counts time two days ahead. Gives the
/// talk, the epochs of its channels and how many sends the front saw before.
fn aged_talk(ids: &[u64], logins: &[&str], retrying: bool) -> (Talk, [u64; 2], usize) {
    let mut talk = Talk::new(ids, logins, retrying);
    with_period(&talk, DAY);
    received(&talk.ada.machine);
    let (before, from) = (epochs(&talk), talk.front.sent().len());
    talk.ada = ahead(talk.ada, 2 * DAY);
    (talk, before, from)
}

/// Starts `skald recv` on Ada's machine and kills it once the front saw it send a Commit in one of
/// the talk's channels after its first `from` sends.
fn killed_after_a_commit(talk: &Talk, from: usize) {
    let started = talk.ada.machine.start(&[RECV]);
    let channels = talk_channels(talk);
    kill_once(started, "a Commit of Ada's machine", || {
        channels
            .iter()
            .any(|channel| commit_sent(&talk.front, *channel, from))
    });
}

/// Ada's `skald recv` commits an empty Commit, which the front swallows, and the run is killed.
/// Meanwhile Bob's device, noticing the aged epochs too, commits an empty Commit in each channel
/// from the same epoch, which the server stores. Ada's next runs, with nothing swallowed, end in
/// Bob's epochs: the server holds one new epoch of each channel, with Ada's and Bob's devices; Bob's
/// device gets no Commit of Ada's; Ada and Bob read each other in the new epoch.
#[test]
fn ada_and_bob_noticing_an_aged_epoch_at_once_give_one_new_epoch() {
    let (mut talk, before, from) = aged_talk(
        &[4_162_204_001, 4_162_204_002],
        &["ada-rotates-at-once", "bob-rotates-at-once"],
        false,
    );
    talk.front.swallow(true);
    killed_after_a_commit(&talk, from);
    let (space, channels) = (talk.space.id, talk_channels(&talk));
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    for channel in channels {
        let empty = (bob.mls)
            .commit_empty(channel, b"", now())
            .expect("an empty Commit");
        bob.commits(cloud, (space, channel), &empty);
        bob.uploads_group_info(cloud, (space, channel));
    }
    talk.front.swallow(false);

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let devices = [talk.ada.device, talk.played[0].device()];
    for (channel, epoch) in next_epochs(&talk, before) {
        group(talk.cloud(), &talk.ada.token, space, channel).assert_is(epoch, &devices);
    }
    let took = talk.played[0].take_in(&talk.world.cloud);
    assert!(
        !took.iter().any(|one| matches!(one, Took::Changed(..))),
        "Bob's device got another Commit: {took:?}"
    );
    for channel in channels {
        assert_members(&talk.played[0].members(channel), &devices);
    }
    assert_talk_in(&mut talk, before[1] + 1, (ADA_SAYS, BOB_SAYS));
    assert_eq!(
        epochs(&talk),
        before.map(|epoch| epoch + 1),
        "not one new epoch"
    );
}

/// How a run loses the answers to Ada's empty Commits.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The front loses each `stored`, and the run ends by itself.
    Stored,
    /// The front loses each `stored`, and the test kills the run once it sent a Commit.
    Killed,
}

/// Ada's aged `skald recv` loses the answers to its empty Commits as `lost` says. Her next runs,
/// with the answers forwarded, leave one new epoch of each channel: the front saw one Commit of
/// hers in each, sent again with the same bytes if at all; Bob's device enters each new epoch
/// once, from her Commit; Ada and Bob read each other in it.
fn one_new_epoch_after_a_lost_answer(lost: Lost, ids: &[u64], logins: &[&str]) {
    let (mut talk, before, from) = aged_talk(ids, logins, true);
    talk.front.answer(Answers::Lose);

    match lost {
        Lost::Stored => {
            drop(run(&talk.ada.machine, &[RECV]));
            assert!(talk.front.lost() >= 1, "no stored was lost");
        }
        Lost::Killed => killed_after_a_commit(&talk, from),
    }
    talk.front.answer(Answers::Forward);
    received(&talk.ada.machine);

    assert_rotated_once(&mut talk, (before, from), (ADA_SAYS, BOB_SAYS));
}

#[test]
fn an_empty_commit_whose_stored_is_lost_leaves_one_new_epoch() {
    one_new_epoch_after_a_lost_answer(
        Lost::Stored,
        &[4_162_205_001, 4_162_205_002],
        &["ada-rotation-lost", "bob-rotation-lost"],
    );
}

#[test]
fn an_empty_commit_of_a_cli_killed_after_sending_it_leaves_one_new_epoch() {
    one_new_epoch_after_a_lost_answer(
        Lost::Killed,
        &[4_162_205_101, 4_162_205_102],
        &["ada-rotation-killed", "bob-rotation-killed"],
    );
}
