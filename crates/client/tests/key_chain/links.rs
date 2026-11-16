//! The links of clean epochs: the sender of each Commit uploads the new epoch's link once the
//! Commit is stored, a member device uploads after its catch-up those of other devices' Commits,
//! and they lead from the latest key back to epoch 1, which has none.

use crate::chain::{
    adds_to_channel, assert_chain, assert_opens, assert_reach, assert_talk_links,
    bob_commits_empty, bob_opens, carol, removes_from_channel,
};
use crate::edits::{Edit, edited};
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

/// Ada's `skald exclude-device` bars Carol's device and commits its Remove: epoch 2. Once the
/// server stores the Commit, the same run uploads the link of epoch 2 with `prev` 1, before any
/// catch-up: Bob's key of epoch 2 opens it to his key of epoch 1.
#[test]
fn sender_of_a_commit_uploads_its_link_once_the_server_stores_it() {
    let mut talk = Talk::new(
        &[4_164_010_001, 4_164_010_002, 4_164_010_003],
        &["ada-links-at-once", "bob-stays-linked", "carol-barred-now"],
        false,
    );

    edited(
        &talk.ada.machine,
        &talk.space.name,
        Edit::Exclude(&talk.played[1].person),
    );

    let ada_and_bob = [talk.ada.device, talk.played[0].device()];
    let held = assert_chain(&talk, (2, &ada_and_bob), &[(2, 1)]);
    bob_opens(&mut talk, &held, (2, 1));
}

/// Bob's device removes Ada's from the channel's group, epoch 2, and adds it again with a Welcome,
/// epoch 3, which Ada's next run joins from, then commits an empty Commit: epoch 4. Ada's run judged
/// epoch 3 clean as it joined: it uploads the link of epoch 3 with `prev` 1, the epoch before it
/// she judged clean, and that of epoch 4 with `prev` 3. Bob's keys open both.
#[test]
fn device_that_joins_from_a_welcome_judges_its_epoch() {
    let mut talk = Talk::new(
        &[4_164_011_001, 4_164_011_002],
        &["ada-welcomed-back", "bob-welcomes-ada"],
        false,
    );
    let (cloud, at) = (&talk.world.cloud, talk.at());
    talk.played[0].removes(cloud, at, talk.ada.device);
    talk.played[0].welcomes(cloud, at, &talk.ada);
    received(&talk.ada.machine);
    bob_commits_empty(&mut talk, 4);

    received(&talk.ada.machine);

    let held = assert_talk_links(&talk, &[(3, 1), (4, 3)]);
    let bob = &talk.played[0];
    for pair in [(3, 1), (4, 3)] {
        assert_opens(&held, talk.channel, pair, (bob, bob));
    }
}
