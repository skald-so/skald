//! The links of clean epochs: the sender of each Commit uploads the new epoch's link once the
//! Commit is stored, a member device uploads after its catch-up those of other devices' Commits,
//! and they lead from the latest key back to epoch 1, which has none.

use crate::chain::{
    adds_to_channel, assert_chain, assert_opens, assert_reach, assert_talk_links,
    bob_commits_empty, carol, removes_from_channel,
};
use crate::kit::{assert_members, received};
use crate::talk::Talk;

/// Ada's device creates the channel's group with Bob's at epoch 1: the server holds no link of the
/// channel. A revision Ada's device signs adds Carol to the channel, and Ada's next run commits
/// the Add of Carol's device: the server holds the link of epoch 2 alone, with `prev` 1, which
/// Carol's key of epoch 2 opens to Bob's key of epoch 1.
#[test]
fn commit_adding_a_device_leaves_the_link_of_the_new_epoch_and_epoch_one_has_none() {
    let talk = Talk::new(
        &[4_164_001_001, 4_164_001_002],
        &["ada-links-epochs", "bob-holds-first-key"],
        false,
    );
    let mut carol = carol(&talk, 4_164_001_003, "carol-opens-link");
    let (ada, bob) = (talk.ada.device, &talk.played[0]);
    assert_chain(&talk, (1, &[ada, bob.device()]), &[]);
    adds_to_channel(talk.cloud(), &talk.ada, talk.at(), &carol.person);

    received(&talk.ada.machine);

    let devices = [ada, bob.device(), carol.device()];
    let held = assert_chain(&talk, (2, &devices), &[(2, 1)]);
    carol.take_in(talk.cloud());
    assert_members(&carol.members(talk.channel), &devices);
    assert_opens(&held, talk.channel, (2, 1), (&carol, bob));
}

/// Over three Commits, Ada's adding Carol, Bob's empty one and Ada's removing Carol, Ada's runs
/// leave the links of epochs 2, 3 and 4, each with the epoch before as `prev`: the one of Bob's
/// Commit after Ada's catch-up. From Bob's key of epoch 4 they reach his keys of epochs 3, 2 and 1.
#[test]
fn links_over_several_commits_lead_from_the_latest_key_back_to_epoch_one() {
    let mut talk = Talk::new(
        &[4_164_002_001, 4_164_002_002],
        &["ada-chains-keys", "bob-follows-chain"],
        false,
    );
    let carol = carol(&talk, 4_164_002_003, "carol-comes-and-goes");
    adds_to_channel(talk.cloud(), &talk.ada, talk.at(), &carol.person);
    received(&talk.ada.machine);
    assert_talk_links(&talk, &[(2, 1)]);
    bob_commits_empty(&mut talk, 3);

    received(&talk.ada.machine);

    assert_talk_links(&talk, &[(2, 1), (3, 2)]);
    removes_from_channel(talk.cloud(), &talk.ada, talk.at(), &carol.person);

    received(&talk.ada.machine);

    let devices = [talk.ada.device, talk.played[0].device()];
    let held = assert_chain(&talk, (4, &devices), &[(2, 1), (3, 2), (4, 3)]);
    talk.played[0].take_in(&talk.world.cloud);
    assert_reach(&held, talk.channel, (&talk.played[0], 4), &[1, 2, 3]);
}
