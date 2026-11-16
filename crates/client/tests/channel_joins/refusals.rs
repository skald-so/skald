//! Refusals around joining by itself: an external Commit the server refuses with `epoch-mismatch`
//! leaves no group, and the device joins once from the latest GroupInfo; a device that refused a
//! Welcome for its signer joins by itself from a GroupInfo that lists it; a GroupInfo member that
//! only a revision the run did not see yet admits is held admitted.

use skald_proto::Role;

use crate::fixtures::cert_of;
use crate::joins::{assert_joined, group_info_bytes, kept, outside, serve_group_info};
use crate::kit::{
    Change, assert_listed_not_in, channels, group, issue_channel, latest, received, revise,
    rosters, stale_once, tell_groups, warnings_naming,
};
use crate::played::{mls_envelope, stranger, take_key_package, welcome_from};
use crate::talk::{Talk, commit_sent};
use crate::world::{ACME, now, warnings};

/// Bob's device commits an empty Commit after the GroupInfo of epoch 1, which the relay keeps
/// giving. Ada's `skald recv` sends an external Commit of epoch 1, which the server refuses with
/// `epoch-mismatch`: it warns of nothing in the channel and lists the channel as one it is not in,
/// and the server's group still holds Bob's device alone. Once the relay gives the server's
/// GroupInfo again, Ada's next run joins by itself at epoch 3, and the run after it sends no Commit
/// in the channel.
#[test]
fn external_commit_refused_with_epoch_mismatch_leaves_no_group_and_the_device_joins_once() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_138_086_001, 4_138_086_002],
        &["ada-joins-too-late", "bob-moves-on"],
    );
    let at = (talk.space.id, channel);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let stale = group_info_bytes(cloud, &bob.person.token, at);
    let empty = bob.mls.commit_empty(channel, b"", now()).expect("a Commit");
    kept(cloud, bob, at, &empty);
    serve_group_info(cloud, at, &stale);
    let before = talk.front.sent().len();

    let refused = received(&talk.ada.machine);

    assert!(
        commit_sent(&talk.front, channel, before),
        "no external Commit sent: {refused:?}"
    );
    let id = channel.to_string();
    assert!(
        warnings(&refused).iter().all(|line| !line.contains(&id)),
        "a warning about the channel: {refused:?}"
    );
    assert_listed_not_in(&channels(&talk.ada.machine, ACME), channel, &[]);
    let bob = &talk.played[0];
    group(cloud, &bob.person.token, at.0, channel).assert_is(2, &[bob.device()]);
    tell_groups(cloud, at.0, channel);
    let later = talk.front.sent().len();

    let joined = received(&talk.ada.machine);

    assert!(
        commit_sent(&talk.front, channel, later),
        "no external Commit sent from the latest GroupInfo: {joined:?}"
    );
    let bob = &mut talk.played[0];
    let held = group(cloud, &bob.person.token, at.0, channel);
    held.assert_is(3, &[bob.device(), ada]);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
    bob.take_in(cloud);
    assert_joined(bob, channel, ada, false);
    let last = talk.front.sent().len();
    let again = received(&talk.ada.machine);
    assert!(
        !commit_sent(&talk.front, channel, last),
        "Ada's device commits again in the channel: {again:?}"
    );
}

/// A revision Bob's device signs adds a channel with Bob and Ada; Bob's device adds Ada's to the
/// channel's group and uploads the GroupInfo, which lists both, but sends no Welcome. A device of
/// no account puts a Welcome of a group of its own into Ada's catch-up. Ada's `skald recv` refuses
/// it, warning naming its signer, and joins by itself from the GroupInfo: the group is at epoch 2
/// with one leaf of each device, and Bob's device gets Ada's as the joiner that removes its old
/// leaf.
#[test]
fn device_that_refused_a_welcome_joins_by_itself_from_a_group_info_that_lists_it() {
    let mut talk = Talk::with(
        ACME,
        &[4_138_086_101, 4_138_086_102],
        &["ada-gets-forged-welcome", "bob-adds-ada"],
        false,
        (Role::Admin, |_| {}),
    );
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let bob = &mut talk.played[0];
    let space = talk.space.id;
    let channel = issue_channel(cloud, &bob.person.token, space);
    let change = Change {
        channels: vec![(channel, vec![&bob.person, &talk.ada])],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &bob.person.token, space),
        &bob.person,
        &change,
    );
    let at = (space, channel);
    bob.mls.create_group(channel, now()).expect("a group");
    let package = take_key_package(cloud, &bob.person.token, ada);
    let commit = (bob.mls)
        .add_devices(channel, &[package], &[cert_of(cloud, &talk.ada)], now())
        .expect("a Commit adding Ada's device");
    bob.commits(cloud, at, &commit);
    bob.uploads_group_info(cloud, at);
    let mut eve = stranger();
    let welcome = welcome_from(cloud, &mut eve, channel, (&talk.ada, &bob.person.token));
    talk.front
        .inject(ada, vec![mls_envelope(at, &welcome, &[ada])]);
    let before = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert_eq!(talk.front.injected(), 1, "not put into catch-up: {ran:?}");
    assert!(
        !warnings_naming(&ran, eve.id()).is_empty(),
        "no warning naming the Welcome's signer: {ran:?}"
    );
    assert!(
        commit_sent(&talk.front, channel, before),
        "no external Commit sent: {ran:?}"
    );
    let bob = &mut talk.played[0];
    let held = group(cloud, &bob.person.token, space, channel);
    held.assert_is(2, &[bob.device(), ada]);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
    bob.take_in(cloud);
    assert_joined(bob, channel, ada, true);
}

/// A revision Bob's device signs adds a channel with Bob and Ada, whose group Bob's device creates
/// alone; the next adds Carol to it, and Bob's device adds Carol's. Ada's run first sees the
/// revisions before Carol joined the channel: it accepts the latest before it holds Carol's device
/// not admitted, so it warns of none and joins by itself at epoch 3.
#[test]
fn group_info_member_that_a_revision_the_run_did_not_see_admits_is_joined_with() {
    let (mut talk, channel) = outside(
        ACME,
        &[4_138_086_201, 4_138_086_202, 4_138_086_203],
        &[
            "ada-sees-late-join",
            "bob-adds-carol-later",
            "carol-joins-late",
        ],
    );
    let at = (talk.space.id, channel);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let (bob, carol) = match talk.played.as_mut_slice() {
        [bob, carol] => (bob, carol),
        _ => unreachable!("Bob's and Carol's devices"),
    };
    let keep = rosters(cloud, &bob.person.token, at.0).len();
    let change = Change {
        join: vec![(channel, &carol.person)],
        ..Change::default()
    };
    revise(
        cloud,
        &latest(cloud, &bob.person.token, at.0),
        &bob.person,
        &change,
    );
    bob.welcomes(cloud, at, &carol.person);
    stale_once(cloud, at.0, keep);
    let (bob, carol) = (bob.device(), carol.device());
    let before = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert!(
        warnings_naming(&ran, carol).is_empty(),
        "Carol's device is held not admitted: {ran:?}"
    );
    assert!(
        commit_sent(&talk.front, channel, before),
        "no external Commit sent: {ran:?}"
    );
    let held = group(cloud, &talk.ada.token, at.0, channel);
    held.assert_is(3, &[bob, carol, ada]);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
}
