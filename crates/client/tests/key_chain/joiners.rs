//! Joins by itself: the joiner has judged no epoch before its join, so it uploads no link of it;
//! the members, which judged the epochs before, upload it.

use skald_proto::Role;

use crate::chain::{
    adds_to_channel, assert_chain, assert_links, assert_opens, assert_talk_links, carol,
    commits_empty, joins_by_itself, links_sent,
};
use crate::joins::outside;
use crate::kit::{group, received};
use crate::talk::Talk;
use crate::world::ACME;

/// Bob's device created another channel's group alone, as no KeyPackage of Ada's device was left:
/// epoch 1. Ada's `skald recv` joins by itself: epoch 2, and Ada's machine sends no link of the
/// channel, and the server holds none. Bob's device then commits an empty Commit: epoch 3. Ada's
/// next run uploads its link with `prev` 2, her join's epoch, as she judged no epoch before it:
/// Bob's key of epoch 3 opens it to his key of epoch 2.
#[test]
fn device_that_joins_by_itself_uploads_no_link_of_its_join_epoch() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_164_004_001, 4_164_004_002],
        &["ada-joins-unlinked", "bob-made-it-alone"],
    );
    let at = (talk.space.id, channel);
    let from = talk.front.uploads().len();

    received(&talk.ada.machine);

    let (cloud, token) = (&talk.world.cloud, &talk.ada.token);
    let bob = &mut talk.played[0];
    group(cloud, token, at.0, at.1).assert_is(2, &[bob.device(), talk.ada.device]);
    let sent = links_sent(&talk.front, channel, from);
    assert!(sent.is_empty(), "the joiner sent links: {sent:?}");
    assert_links(cloud, token, at, &[]);
    let started = commits_empty(cloud, bob, at);
    assert_eq!(started, 3, "the epoch of Bob's empty Commit");

    received(&talk.ada.machine);

    let held = assert_links(cloud, token, at, &[(3, 2)]);
    let bob = &talk.played[0];
    assert_opens(&held, channel, (3, 2), (bob, bob));
}

/// A revision Bob's device signs adds Carol to the channel, and Carol's device joins its group by
/// itself: epoch 2. Ada's device did not commit it, but her next run uploads its link with `prev`
/// 1, which Carol's key of epoch 2 opens to Bob's key of epoch 1.
#[test]
fn members_upload_the_link_of_the_epoch_another_device_joined_by_itself() {
    let talk = Talk::with(
        ACME,
        &[4_164_005_001, 4_164_005_002],
        &["ada-links-joiner", "bob-admits-carol"],
        false,
        (Role::Admin, |_| {}),
    );
    let mut carol = carol(&talk, 4_164_005_003, "carol-joins-herself");
    let bob = &talk.played[0];
    adds_to_channel(talk.cloud(), &bob.person, talk.at(), &carol.person);
    let joined = joins_by_itself(talk.cloud(), &mut carol, talk.at());
    assert_eq!(joined, 2, "the epoch of Carol's join");
    assert_talk_links(&talk, &[]);

    received(&talk.ada.machine);

    let devices = [talk.ada.device, bob.device(), carol.device()];
    let held = assert_chain(&talk, (2, &devices), &[(2, 1)]);
    assert_opens(&held, talk.channel, (2, 1), (&carol, bob));
}
