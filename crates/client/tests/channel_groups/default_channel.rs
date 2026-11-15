//! The default channel: every space has one, the first channel the roster lists, with every member
//! of the space, and its group.

use std::thread;
use std::time::Duration;

use skald_proto::{ChannelId, Role, SpaceId};

use crate::creating::{ADA_HOST, BOB_HOST};
use crate::fixtures::fronted;
use crate::kit::{
    Change, assert_adds_default, assert_listed_with, assert_with_every_member, channels,
    create_older_space, create_space, group, issue_channel, latest, received, revise, rosters,
    stale_once,
};
use crate::machine::Finished;
use crate::relay::{Lose, strings_named};
use crate::world::{
    ACME, CREATE_CHANNEL, Person, World, next_with, run, space_id, store, warnings,
};

/// The revision after `previous`, signed by `signer`'s device and stored as a fixture, that adds
/// `added` as a member with its role, outside any channel.
fn joins_outside_default(
    cloud: &crate::cloud::Cloud,
    previous: &skald_proto::Roster,
    signer: &Person,
    added: (&Person, Role),
) -> skald_proto::Roster {
    let change = Change {
        members: vec![added],
        outside_default: true,
        ..Change::default()
    };
    revise(cloud, previous, signer, &change)
}

/// Fails unless `channel`'s group, as the server's GroupInfo shows it to `person`'s session, is at
/// epoch 1 with `devices`, and `person`'s machine lists the channel with the device names `names`.
#[track_caller]
fn assert_formed(
    cloud: &crate::cloud::Cloud,
    person: &Person,
    (space, name): (SpaceId, &str),
    channel: ChannelId,
    (devices, names): (&[skald_proto::DeviceId], &[&str]),
) {
    group(cloud, &person.token, space, channel).assert_is(1, devices);
    assert_listed_with(&channels(&person.machine, name), channel, names, &[]);
}

/// A new team space has a default channel, the first the roster lists, from the revision after
/// revision 1, which Ada's device signs; its group, which Ada's device forms, holds that device.
#[test]
fn new_team_space_has_a_default_channel_with_its_creator_in_its_group() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_001, "ada-default-team", ADA_HOST);

    let space = create_space(cloud, &ada, ACME);

    let held = latest(cloud, &ada.token, space.id);
    assert_eq!(held.body().channels[0].id, space.default);
    assert_formed(
        cloud,
        &ada,
        (space.id, ACME),
        space.default,
        (&[ada.device], &[ADA_HOST]),
    );
}

/// The personal space a new account's first machine takes has a default channel too, with that
/// machine's device in its group.
#[test]
fn new_personal_space_has_a_default_channel_with_its_device_in_its_group() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_101, "ada-default-own", ADA_HOST);
    let personal = ada.handle.to_string();
    received(&ada.machine);

    let space = space_id(cloud, &ada.token, &personal);
    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 2, "the personal space's revisions: {held:?}");
    let default = assert_adds_default(cloud, &held[0], &held[1], &ada);
    assert_formed(
        cloud,
        &ada,
        (space, &personal),
        default,
        (&[ada.device], &[ADA_HOST]),
    );
}

/// A team space with revision 1 alone, as one made before default channels, gets its default
/// channel at the next run of its admin's device, which forms its group.
#[test]
fn space_with_revision_1_alone_gets_a_default_channel_at_the_next_run() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_201, "ada-older-team", ADA_HOST);
    let (space, first) = create_older_space(cloud, &ada, ACME);

    received(&ada.machine);

    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 2, "the space's revisions: {held:?}");
    assert_eq!(held[0], first, "revision 1 changed");
    let default = assert_adds_default(cloud, &first, &held[1], &ada);
    assert_formed(
        cloud,
        &ada,
        (space, ACME),
        default,
        (&[ada.device], &[ADA_HOST]),
    );
}

/// A personal space with revision 1 alone, as one made before default channels, gets its default
/// channel at the next run of its device.
#[test]
fn personal_space_with_revision_1_alone_gets_a_default_channel_at_the_next_run() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    crate::kit::refuse_later_revisions(cloud);
    let ada = world.sign_up(4_135_051_301, "ada-older-own", ADA_HOST);
    crate::kit::store_later_revisions(cloud);
    let personal = ada.handle.to_string();
    let space = space_id(cloud, &ada.token, &personal);
    crate::kit::forget_waiting(&ada.machine, space);
    let first = rosters(cloud, &ada.token, space);
    assert_eq!(first.len(), 1, "not revision 1 alone: {first:?}");

    received(&ada.machine);

    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 2, "the personal space's revisions: {held:?}");
    assert_eq!(held[0], first[0], "revision 1 changed");
    let default = assert_adds_default(cloud, &first[0], &held[1], &ada);
    assert_formed(
        cloud,
        &ada,
        (space, &personal),
        default,
        (&[ada.device], &[ADA_HOST]),
    );
}

/// A space whose revision 2, a fixture Ada's device signs, adds Bob before any channel gets a
/// default channel with both accounts, and its group with both devices.
#[test]
fn space_whose_revision_adds_a_member_before_any_channel_gets_one_with_both() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_401, "ada-two-first", ADA_HOST);
    let bob = world.sign_up(4_135_051_402, "bob-two-first", BOB_HOST);
    let (space, first) = create_older_space(cloud, &ada, ACME);
    let second = joins_outside_default(cloud, &first, &ada, (&bob, Role::Member));

    received(&ada.machine);

    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 3, "the space's revisions: {held:?}");
    let default = assert_adds_default(cloud, &second, &held[2], &ada);
    assert_formed(
        cloud,
        &ada,
        (space, ACME),
        default,
        (&[ada.device, bob.device], &[ADA_HOST, BOB_HOST]),
    );
}

/// Bob, added to `acme` by a fixture revision outside its default channel, leaves that channel
/// without a member: Ada's device signs a new default channel with both accounts first in the
/// roster, keeps the old one second, and forms the new one's group with both devices.
#[test]
fn space_whose_first_channel_lacks_a_member_gets_a_new_default_channel_first() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_501, "ada-lacking", ADA_HOST);
    let bob = world.sign_up(4_135_051_502, "bob-left-out", BOB_HOST);
    let space = create_space(cloud, &ada, ACME);
    let previous = latest(cloud, &ada.token, space.id);
    let third = joins_outside_default(cloud, &previous, &ada, (&bob, Role::Member));

    received(&ada.machine);

    let held = rosters(cloud, &ada.token, space.id);
    assert_eq!(held.len(), 4, "the space's revisions: {held:?}");
    let default = assert_adds_default(cloud, &third, &held[3], &ada);
    assert_eq!(held[3].body().channels[1].id, space.default, "the old one");
    assert_formed(
        cloud,
        &ada,
        (space.id, ACME),
        default,
        (&[ada.device, bob.device], &[ADA_HOST, BOB_HOST]),
    );
}

/// `skald create-channel` adds a channel after the default one, which stays first, and forms the
/// new channel's group at once, with Ada's device.
#[test]
fn channel_created_later_goes_after_the_default_channel_with_its_group_formed_at_once() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_601, "ada-second-channel", ADA_HOST);
    let space = create_space(cloud, &ada, ACME);

    let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);

    assert!(created.success, "{created:?}");
    let held = rosters(cloud, &ada.token, space.id);
    assert_eq!(held.len(), 3, "the space's revisions: {held:?}");
    let channels_held = &held[2].body().channels;
    assert_eq!(channels_held.len(), 2, "{channels_held:?}");
    assert_eq!(
        channels_held[0],
        held[1].body().channels[0],
        "the default one"
    );
    let added = channels_held[1].id;
    assert!(created.shown().contains(&added.to_string()), "{created:?}");
    group(cloud, &ada.token, space.id, added).assert_is(1, &[ada.device]);
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, space.default, &[ADA_HOST], &[]);
    assert_listed_with(&listed, added, &[ADA_HOST], &[]);
}

/// The request that stores a revision of `space`, and the one that issues a channel ID for it, as
/// their lines start.
fn store_request(space: SpaceId) -> String {
    format!("POST /spaces/{space}/revisions")
}
fn channel_request(space: SpaceId) -> String {
    format!("POST /spaces/{space}/channels")
}

/// Fails unless `space` has revision 1 and one revision 2 with one channel, the default one with
/// Ada's device in its group, which Ada's machine lists alone.
#[track_caller]
fn assert_one_default_channel(world: &World, ada: &Person, space: SpaceId, first: &[u8]) {
    let cloud = &world.cloud;
    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 2, "the space's revisions: {held:?}");
    assert_eq!(skald_proto::Cbor::to_cbor(&held[0]), first, "revision 1");
    let default = assert_adds_default(cloud, &held[0], &held[1], ada);
    assert_eq!(held[1].body().channels.len(), 1);
    group(cloud, &ada.token, space, default).assert_is(1, &[ada.device]);
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, default, &[ADA_HOST], &[]);
    let ids: Vec<String> = strings_named(&cloud.relay.answered(), "channel")
        .into_iter()
        .filter(|id| *id != default.to_string())
        .collect();
    for id in ids {
        let id: ChannelId = id.parse().expect("a channel ID");
        assert!(
            crate::kit::lines_of(&listed, id).is_empty(),
            "another channel {id} is listed: {listed:?}"
        );
    }
}

/// Which answer between revision 1 and revision 2 is lost, and how.
#[derive(Clone, Copy)]
enum Between {
    /// The answer to revision 2, the connection closed.
    RevisionDropped,
    /// The answer to revision 2, withheld, and the CLI killed.
    RevisionKilled,
    /// The answer with the default channel's ID, withheld, and the CLI killed.
    ChannelIdKilled,
}

/// Ada's run that signs the default channel of a space with revision 1 alone loses an answer as
/// `between` says; the next run leaves exactly one default channel.
fn one_default_channel_after(between: Between, (id, login): (u64, &str)) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = crate::fixtures::retrying_for_the_timeout(world.sign_up(id, login, ADA_HOST));
    let (space, first) = create_older_space(cloud, &ada, ACME);
    let (request, how) = match between {
        Between::RevisionDropped => (store_request(space), Lose::Close),
        Between::RevisionKilled => (store_request(space), Lose::Withhold),
        Between::ChannelIdKilled => (channel_request(space), Lose::Withhold),
    };
    cloud.relay.lose_answer(&request, how);

    let started = ada.machine.start(&[crate::kit::RECV]);
    match how {
        Lose::Close => drop(started.finish()),
        Lose::Withhold => cloud.kill_once_lost(started, &request),
    }
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    // A revision signed again now has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));
    received(&ada.machine);

    assert_one_default_channel(&world, &ada, space, &skald_proto::Cbor::to_cbor(&first));
}

#[test]
fn revision_answer_dropped_between_revisions_1_and_2_leaves_one_default_channel() {
    one_default_channel_after(
        Between::RevisionDropped,
        (4_135_051_701, "ada-lost-default"),
    );
}

#[test]
fn cli_killed_waiting_for_revision_2_leaves_one_default_channel() {
    one_default_channel_after(
        Between::RevisionKilled,
        (4_135_051_702, "ada-killed-default"),
    );
}

#[test]
fn cli_killed_waiting_for_the_default_channel_id_leaves_one_default_channel() {
    one_default_channel_after(
        Between::ChannelIdKilled,
        (4_135_051_703, "ada-killed-default-id"),
    );
}

/// Ada and Bob, both admins of `acme`, which has no channel yet: Bob's device stores a revision
/// with the default channel while Ada's device, which first sees the revisions as they were before
/// it, signs one too. The server keeps Bob's; Ada's device accepts it and signs nothing more, so the
/// space has one channel, and Ada's machine lists it.
#[test]
fn default_channel_two_admins_sign_at_once_is_kept_once() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_051_801, "ada-at-once", ADA_HOST);
    let bob = world.sign_up(4_135_051_802, "bob-at-once", BOB_HOST);
    let (space, first) = create_older_space(cloud, &ada, ACME);
    let second = joins_outside_default(cloud, &first, &ada, (&bob, Role::Admin));
    let bobs = issue_channel(cloud, &bob.token, space);
    let third = next_with(&second, (bob.device, &bob.key), |body| {
        body.channels.push(skald_proto::Channel {
            id: bobs,
            members: [&ada, &bob]
                .iter()
                .map(|person| skald_proto::ChannelMember {
                    account: person.account,
                    history: true,
                })
                .collect(),
            services: Vec::new(),
            unknown: skald_proto::Unknown::default(),
        });
    });
    store(cloud, &bob.token, &third);
    stale_once(cloud, space, 2);
    let since = cloud.relay.mark();

    let ran: Finished = received(&ada.machine);

    assert!(
        cloud.count_sent(since, &store_request(space)) >= 1,
        "Ada's device signed no default channel: {:?}",
        cloud.relay.sent_since(since)
    );
    assert!(
        !warnings(&ran).iter().any(|line| line.contains("fork")),
        "its own refused revision is taken for a fork: {ran:?}"
    );
    let held = rosters(cloud, &ada.token, space);
    assert_eq!(held.len(), 3, "the space's revisions: {held:?}");
    assert_eq!(held[2], third, "Bob's revision is not kept");
    assert_with_every_member(held[2].body(), &held[2].body().channels[0]);
    let listed = channels(&ada.machine, ACME);
    assert_eq!(
        crate::kit::lines_of(&listed, bobs).len(),
        1,
        "Bob's channel: {listed:?}"
    );
    for id in strings_named(&cloud.relay.answered_since(since), "channel") {
        let id: ChannelId = id.parse().expect("a channel ID");
        assert!(
            crate::kit::lines_of(&listed, id).is_empty(),
            "Ada's own default channel {id} is listed: {listed:?}"
        );
    }
}
