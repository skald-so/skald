//! Joining by itself: Ada's device, admitted to a channel whose group it is not in, joins with an
//! external Commit from the latest GroupInfo and uploads the new epoch's GroupInfo, then reads and
//! sends there; a GroupInfo that lists a device that is not admitted gives no join and a warning.

use skald_proto::Role;

use crate::fixtures::{fronted, person_address, send};
use crate::joins::{
    assert_joined, forked_group_info, intruder, outside, read_from, serve_group_info,
};
use crate::kit::{
    Change, assert_listed_not_in, assert_listed_with, assert_members, channels, create_space,
    group, issue_channel, latest, received, revise, tell_groups, warnings_naming,
};
use crate::played::Played;
use crate::talk::{
    ADA_HOST, BOB_HOST, CAROL_HOST, Line, assert_shows_once, commit_sent, encrypted, stored,
};
use crate::world::ACME;

/// The texts the tests send; none is anywhere else.
const SAYS: [&str; 4] = [
    "joined the launch channel, catching up",
    "welcome, the checklist is pinned",
    "the venue confirmed the late checkout",
    "great, I will tell the speakers",
];

/// Bob, an admin outside the channel, signs a revision that adds a channel with Carol alone, whose
/// group Carol's device creates; then one that adds Ada to it. Ada's `skald recv` joins by itself:
/// it sends an external Commit and uploads the GroupInfo of epoch 2, which lists Carol's and Ada's
/// devices. Carol's device gets Ada's device as the joiner, and `skald channels` lists the channel
/// with both. Ada's `skald send` reaches Carol's device, and Carol's message shows on Ada's next
/// run.
#[test]
fn device_added_by_an_admin_outside_the_channel_joins_by_itself_and_reads_and_sends() {
    let (world, front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_138_081_001, "ada-joins-herself", ADA_HOST);
    let bob = world.sign_up(4_138_081_002, "bob-admin-aside", BOB_HOST);
    let mut carol = Played::new(
        cloud,
        world.sign_up(4_138_081_003, "carol-holds-channel", CAROL_HOST),
    );
    let space = create_space(cloud, &ada, ACME);
    let previous = latest(cloud, &ada.token, space.id);
    let change = Change {
        members: vec![(&bob, Role::Admin), (&carol.person, Role::Member)],
        ..Change::default()
    };
    revise(cloud, &previous, &ada, &change);
    received(&ada.machine);
    carol.take_in(cloud);
    let channel = issue_channel(cloud, &bob.token, space.id);
    let previous = latest(cloud, &bob.token, space.id);
    let change = Change {
        channels: vec![(channel, vec![&carol.person])],
        ..Change::default()
    };
    let alone = revise(cloud, &previous, &bob, &change);
    let at = (space.id, channel);
    carol.creates_alone(cloud, at);
    carol.uploads_group_info(cloud, at);
    let change = Change {
        join: vec![(channel, &ada)],
        ..Change::default()
    };
    revise(cloud, &alone, &bob, &change);
    let from = front.sent().len();

    let ran = received(&ada.machine);

    assert!(
        commit_sent(&front, channel, from),
        "no external Commit sent: {ran:?}"
    );
    let held = group(cloud, &ada.token, space.id, channel);
    held.assert_is(2, &[carol.device(), ada.device]);
    assert_eq!(held.signer, ada.device, "the GroupInfo's signer");
    carol.take_in(cloud);
    assert_joined(&carol, channel, ada.device, false);
    assert_members(&carol.members(channel), &[carol.device(), ada.device]);
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, CAROL_HOST], &[BOB_HOST]);
    let said = send(&ada.machine, channel, SAYS[0]);
    assert!(said.success, "{said:?}");
    carol.take_in(cloud);
    assert_eq!(
        read_from(&carol, channel, SAYS[0]),
        [ada.device],
        "Carol's device does not read Ada's message once"
    );
    let from_carol = person_address(&space.name, &carol.person);
    let envelope = encrypted(&mut carol, at, channel, &Line::new(&from_carol, 1, SAYS[1]));
    stored(cloud, &carol, &envelope);

    let shown = received(&ada.machine).shown();

    assert_shows_once(&shown, SAYS[1], (channel, &from_carol, CAROL_HOST));
}

/// Bob's device created the channel's group alone, as no KeyPackage of Ada's device was left. Ada's
/// `skald recv` joins by itself: the group is at epoch 2 with Bob's device and one leaf of Ada's,
/// as the GroupInfo Ada's device uploads and Bob's device show; Bob's device gets Ada's as the
/// joiner. Ada's next run sends no Commit in the channel. Bob's message shows on Ada's machine, and
/// Ada's reaches Bob's device.
#[test]
fn device_the_creator_got_no_key_package_for_joins_by_itself_with_one_leaf() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_138_081_101, 4_138_081_102],
        &["ada-without-packages", "bob-creates-alone"],
    );
    let at = (talk.space.id, channel);
    let ada = talk.ada.device;
    let before = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert!(
        commit_sent(&talk.front, channel, before),
        "no external Commit sent: {ran:?}"
    );
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let held = group(cloud, &bob.person.token, at.0, channel);
    held.assert_is(2, &[bob.device(), ada]);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
    bob.take_in(cloud);
    assert_joined(bob, channel, ada, false);
    assert_members(&bob.members(channel), &[bob.device(), ada]);
    let later = talk.front.sent().len();
    let again = received(&talk.ada.machine);
    assert!(
        !commit_sent(&talk.front, channel, later),
        "Ada's device commits again in the channel: {again:?}"
    );
    let bob = &mut talk.played[0];
    group(cloud, &bob.person.token, at.0, channel).assert_is(2, &[bob.device(), ada]);
    let from_bob = person_address(&talk.space.name, &bob.person);
    let envelope = encrypted(bob, at, channel, &Line::new(&from_bob, 1, SAYS[2]));
    stored(cloud, bob, &envelope);

    let shown = received(&talk.ada.machine).shown();

    assert_shows_once(&shown, SAYS[2], (channel, &from_bob, BOB_HOST));
    let said = send(&talk.ada.machine, channel, SAYS[3]);
    assert!(said.success, "{said:?}");
    let bob = &mut talk.played[0];
    bob.take_in(cloud);
    assert_eq!(
        read_from(bob, channel, SAYS[3]),
        [ada],
        "Bob's device does not read Ada's message once"
    );
}

/// The relay gives, as the channel's GroupInfo, that of a fork signed by Bob's device that lists a
/// device the members do not admit. Ada's `skald recv` sends no Commit in the channel, warns naming
/// that device, and `skald channels` lists the channel as one Ada's device is not in; the server's
/// group still holds Bob's device alone. Once the relay gives the server's GroupInfo again, Ada's
/// next run joins by itself, and Bob's device gets Ada's as the joiner.
#[test]
fn group_info_listing_a_device_that_is_not_admitted_gives_no_join_and_a_warning() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_138_081_201, 4_138_081_202],
        &["ada-refuses-fork", "bob-honest-creator"],
    );
    let at = (talk.space.id, channel);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let stranger = intruder(cloud, &bob.person, "bob-stranger");
    serve_group_info(
        cloud,
        at,
        &forked_group_info(&bob.person, channel, &stranger),
    );
    let before = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert!(
        !warnings_naming(&ran, stranger.id()).is_empty(),
        "no warning naming the device that is not admitted: {ran:?}"
    );
    assert!(
        !commit_sent(&talk.front, channel, before),
        "a Commit sent from the fork's GroupInfo: {ran:?}"
    );
    let bob = &mut talk.played[0];
    group(cloud, &bob.person.token, at.0, channel).assert_is(1, &[bob.device()]);
    assert_listed_not_in(&channels(&talk.ada.machine, ACME), channel, &[]);
    tell_groups(cloud, at.0, channel);
    let later = talk.front.sent().len();

    let next = received(&talk.ada.machine);

    assert!(
        commit_sent(&talk.front, channel, later),
        "no external Commit sent once the GroupInfo passes: {next:?}"
    );
    let bob = &mut talk.played[0];
    group(cloud, &bob.person.token, at.0, channel).assert_is(2, &[bob.device(), ada]);
    bob.take_in(cloud);
    assert_joined(bob, channel, ada, false);
}
