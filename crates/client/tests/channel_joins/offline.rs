//! A device offline past the mailbox retention: the Commits it missed do not expire, so it
//! processes them in order and stays in the group; gap detection reports the messages that expired
//! once a later one from the same device arrives.

use std::time::Duration;

use skald_proto::Role;
use skald_server::Settings;

use crate::fixtures::cert_of;
use crate::gaps::{assert_none_reported, assert_reported_once, bob_in, bob_sends, marked};
use crate::joins::read_from;
use crate::kit::{Change, assert_listed_with, channels, group, latest, received, revise};
use crate::played::{Played, take_key_package};
use crate::talk::{ADA_HOST, BOB_HOST, CAROL_HOST, Talk, assert_shows_once};
use crate::times::{MAILBOX, WEEK, moved_back};
use crate::world::{ACME, now};

/// The texts the tests send; none is anywhere else.
const SAYS: [&str; 4] = [
    "the archive migration starts monday",
    "carol is helping with the archive this week",
    "the archive migration is done",
    "back online, reading up",
];

/// Gives the server a mailbox retention of 7 days.
fn retained(settings: &mut Settings) {
    settings.mailbox_retention = Some(Duration::from_secs(WEEK));
}

/// With a retention of 7 days: Ada's `skald recv` shows Bob's first message, then her machine stays
/// offline. A revision Bob's device signs adds Carol to the space and the channel, and Bob's device
/// adds Carol's; Bob's device sends a message; a revision takes Carol out of the channel, and Bob's
/// device removes hers. With every storing time moved back 8 days, Ada's next run shows none of the
/// expired message and reports nothing yet; it processes both Commits: the group stays at the epoch
/// of Bob's last Commit, Bob's device gets no Commit of Ada's, and `skald channels` lists Ada's and
/// Bob's devices in the channel, not Carol's. Bob's next message shows on Ada's next run, marked,
/// with messages reported missing once; Ada's message then reaches Bob's device.
#[test]
fn device_offline_past_the_retention_processes_the_commits_it_missed_and_reports_expired_messages()
{
    let mut talk = Talk::with(
        ACME,
        &[4_138_084_001, 4_138_084_002],
        &["ada-long-offline", "bob-carries-on"],
        false,
        (Role::Admin, retained),
    );
    let (at, channel) = (talk.at(), talk.channel);
    let ada = talk.ada.device;
    let (from_bob, _, bob) = bob_in(&talk);
    let carol = Played::new(
        &talk.world.cloud,
        talk.world
            .sign_up(4_138_084_003, "carol-briefly-in", CAROL_HOST),
    );
    bob_sends(&mut talk, 1, SAYS[0]);
    let shown = received(&talk.ada.machine).shown();
    assert_shows_once(&shown, SAYS[0], (channel, &from_bob, BOB_HOST));
    let cloud = &talk.world.cloud;
    let played = &mut talk.played[0];
    let previous = latest(cloud, &played.person.token, at.0);
    let change = Change {
        members: vec![(&carol.person, Role::Member)],
        join: vec![(channel, &carol.person)],
        ..Change::default()
    };
    let with_carol = revise(cloud, &previous, &played.person, &change);
    let package = take_key_package(cloud, &played.person.token, carol.device());
    let added = (played.mls)
        .add_devices(channel, &[package], &[cert_of(cloud, &carol.person)], now())
        .expect("a Commit adding Carol's device");
    let welcome = played.commits(cloud, at, &added).expect("a Welcome");
    played.uploads_group_info(cloud, at);
    played.stores(cloud, at, &welcome, &[carol.device()]);
    bob_sends(&mut talk, 2, SAYS[1]);
    let cloud = &talk.world.cloud;
    let played = &mut talk.played[0];
    let change = Change {
        leave: vec![(channel, &carol.person)],
        ..Change::default()
    };
    revise(cloud, &with_carol, &played.person, &change);
    let removed = (played.mls)
        .remove_devices(channel, &[carol.device()], b"", now())
        .expect("a Commit removing Carol's device");
    played.commits(cloud, at, &removed);
    played.uploads_group_info(cloud, at);
    cloud.execute(&moved_back(&MAILBOX, "8 days"));

    let back = received(&talk.ada.machine).shown();

    assert!(!back.contains(SAYS[1]), "the expired message: {back}");
    assert_none_reported(&back);
    let cloud = &talk.world.cloud;
    let held = group(cloud, &talk.ada.token, at.0, channel);
    held.assert_is(3, &[ada, bob]);
    let played = &mut talk.played[0];
    played.take_in(cloud);
    let from_ada: Vec<_> = (played.changes_of(channel).into_iter())
        .filter(|change| change.sender == ada)
        .collect();
    assert!(
        from_ada.is_empty(),
        "Ada's device committed rather than processing what it missed: {from_ada:?}"
    );
    let listed = channels(&talk.ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, BOB_HOST], &[CAROL_HOST]);
    let third = bob_sends(&mut talk, 3, SAYS[2]);

    let caught_up = received(&talk.ada.machine).shown();

    assert_reported_once(&caught_up, channel, bob);
    assert_shows_once(&caught_up, SAYS[2], (channel, &from_bob, BOB_HOST));
    assert!(
        marked(&caught_up, third),
        "the message after the expired one not marked: {caught_up}"
    );
    let said = talk.send(SAYS[3]);
    assert!(said.success, "{said:?}");
    let played = &mut talk.played[0];
    played.take_in(&talk.world.cloud);
    assert_eq!(
        read_from(played, channel, SAYS[3]),
        [ada],
        "Bob's device does not read Ada's message once"
    );
}
