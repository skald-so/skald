//! What a device does with its channels' groups beyond the tests of channel groups
//! (`docs/protocol/membership.md`, "Admitted devices", "Group changes" and "MLS profile"): before it
//! holds a device a Commit adds, or a Welcome's signer, not admitted, it accepts the space's latest
//! revisions, so a device that a revision its run did not see yet admits is taken as admitted; a
//! run brings in line every channel whose Commits it processes, not only the one it sends to; and a
//! run uploads the GroupInfo of a group it holds when the server has none of its epoch.
//!
//! As the tests of channel groups (`channel_groups/main.rs`): the CLI under test runs on Ada's
//! machine, the other people's devices are played by the tests, and the first answer the relay
//! gives to a request for the space's revisions holds only the revisions before the one that admits
//! the device, as a run that fetched them before it was stored sees them.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../channel_groups/played.rs"]
mod played;

#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../roster_revisions/world.rs"]
mod world;

#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

use skald_proto::Role;

use fixtures::{cert_of, fronted, send};
use kit::{
    Change, assert_listed_with, channels, create_space, group, group_on_server, issue_channel,
    latest, received, revise, stale_once, warnings_naming,
};
use played::{Played, take_key_package};
use world::{ACME, now};

/// The host names of the people's machines, their certificates' `name`.
const ADA_HOST: &str = "ada-workstation";
const BOB_HOST: &str = "bob-workstation";
const CAROL_HOST: &str = "carol-laptop";

/// `acme`, which Ada's machine creates, and its revision 3, which Ada's device signs: it adds
/// `added` with their roles, and a new channel of Ada and the first of them; gives the space, the
/// channel and the revision.
fn acme_with_channel(
    world: &world::World,
    ada: &world::Person,
    added: &[(&world::Person, Role)],
) -> (kit::Space, skald_proto::ChannelId, skald_proto::Roster) {
    let cloud = &world.cloud;
    let space = create_space(cloud, ada, ACME);
    let channel = issue_channel(cloud, &ada.token, space.id);
    let change = Change {
        members: added.to_vec(),
        channels: vec![(channel, vec![ada, added[0].0])],
        ..Change::default()
    };
    let third = revise(cloud, &latest(cloud, &ada.token, space.id), ada, &change);
    (space, channel, third)
}

/// Fails unless the server's GroupInfo of `channel` in `space` is of `epoch`, its members `devices`,
/// and Ada's device signed it.
#[track_caller]
fn assert_ada_signs(
    cloud: &cloud::Cloud,
    ada: &world::Person,
    (space, channel): (skald_proto::SpaceId, skald_proto::ChannelId),
    (epoch, devices): (u64, &[skald_proto::DeviceId]),
) {
    let held = group(cloud, &ada.token, space, channel);
    held.assert_is(epoch, devices);
    assert_eq!(held.signer, ada.device, "the GroupInfo's signer");
}

/// Ada's machine creates `acme`, whose revision 3, by Ada's device, adds Bob as an admin, Carol as a
/// member and a channel of Ada and Bob, whose group Ada's device forms with Bob's; revision 4, by
/// Bob's device, adds Carol to the channel, and, if `leaves`, revision 5 takes her out again. Bob's
/// device then commits the Add of Carol's in the channel. Gives the space and the channel.
fn bob_adds_carol(
    world: &world::World,
    ada: &world::Person,
    (bob, carol): (&mut Played, &Played),
    leaves: bool,
) -> (kit::Space, skald_proto::ChannelId) {
    let cloud = &world.cloud;
    let added = [(&bob.person, Role::Admin), (&carol.person, Role::Member)];
    let (space, channel, third) = acme_with_channel(world, ada, &added);
    received(&ada.machine);
    bob.take_in(cloud);
    let joins = Change {
        join: vec![(channel, &carol.person)],
        ..Change::default()
    };
    let fourth = revise(cloud, &third, &bob.person, &joins);
    if leaves {
        let leave = Change {
            leave: vec![(channel, &carol.person)],
            ..Change::default()
        };
        revise(cloud, &fourth, &bob.person, &leave);
    }
    let package = take_key_package(cloud, &bob.person.token, carol.device());
    let carol_cert = cert_of(cloud, &carol.person);
    let commit = (bob.mls)
        .add_devices(channel, &[package], &[carol_cert], now())
        .expect("a Commit");
    bob.commits(cloud, (space.id, channel), &commit);
    (space, channel)
}

/// Bob's device commits the Add of Carol's, whom revision 4 adds to the channel. Ada's run first
/// sees the revisions up to 3: it accepts revision 4 before it holds Carol's device not admitted, so
/// it warns of none and keeps it, and uploads the new epoch's GroupInfo.
#[test]
fn device_a_commit_adds_that_a_revision_the_run_did_not_see_admits_stays() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_056_001, "ada-sees-late", ADA_HOST);
    let mut bob = Played::new(
        cloud,
        world.sign_up(4_135_056_002, "bob-adds-carol", BOB_HOST),
    );
    let carol = Played::new(
        cloud,
        world.sign_up(4_135_056_003, "carol-admitted-late", CAROL_HOST),
    );
    let (space, channel) = bob_adds_carol(&world, &ada, (&mut bob, &carol), false);
    stale_once(cloud, space.id, 3);

    let ran = received(&ada.machine);

    assert!(
        warnings_naming(&ran, carol.device()).is_empty(),
        "Carol's device is held not admitted: {ran:?}"
    );
    assert_ada_signs(
        cloud,
        &ada,
        (space.id, channel),
        (2, &[ada.device, bob.device(), carol.device()]),
    );
}

/// Bob's device commits the Add of Carol's in the channel, which revision 5 takes her out of. Ada's
/// `skald send` to the default channel goes through that Commit too: it warns of Carol's device,
/// commits its Remove from the channel's group and uploads the new epoch's GroupInfo.
#[test]
fn send_to_one_channel_removes_a_device_another_channels_commit_added_that_is_not_admitted() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_056_201, "ada-sends-elsewhere", ADA_HOST);
    let mut bob = Played::new(
        cloud,
        world.sign_up(4_135_056_202, "bob-lets-carol-in", BOB_HOST),
    );
    let carol = Played::new(
        cloud,
        world.sign_up(4_135_056_203, "carol-let-in-late", CAROL_HOST),
    );
    let (space, channel) = bob_adds_carol(&world, &ada, (&mut bob, &carol), true);

    let sent = send(&ada.machine, space.default, "the standup moves to ten");

    assert!(sent.success, "{sent:?}");
    assert!(
        !warnings_naming(&sent, carol.device()).is_empty(),
        "no warning naming Carol's device: {sent:?}"
    );
    assert_ada_signs(
        cloud,
        &ada,
        (space.id, channel),
        (3, &[ada.device, bob.device()]),
    );
}

/// The request that uploads a channel's GroupInfo, as its line starts.
const UPLOAD: &str = "PUT /spaces/";

/// The server refuses the GroupInfo that Ada's `skald create-space` uploads for the default
/// channel's group, which it formed alone. Ada's next run, `skald channels`, uploads the GroupInfo
/// of epoch 1, since the server has none.
#[test]
fn group_info_whose_upload_failed_is_uploaded_by_the_next_run() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_056_301, "ada-uploads-again", ADA_HOST);
    cloud.relay.refuse(UPLOAD, "503 Service Unavailable");
    let space = create_space(cloud, &ada, ACME);
    cloud.relay.stop_refusing();
    let before = group_on_server(cloud, &ada.token, space.id, space.default);
    assert!(
        matches!(before, (true, None)),
        "not a group without GroupInfo: {before:?}"
    );

    channels(&ada.machine, ACME);

    assert_ada_signs(cloud, &ada, (space.id, space.default), (1, &[ada.device]));
}

/// Revision 3, by Ada's device, adds Bob as a member and a channel of Ada and Bob, which Bob's
/// device forms with Ada's and welcomes it to. Ada's run first sees the revisions up to 2, in which
/// Bob is admitted to no channel: it accepts revision 3 before it refuses the Welcome, so it joins
/// the group and lists the channel with both devices.
#[test]
fn welcome_from_a_device_that_a_revision_the_run_did_not_see_admits_is_joined() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_135_056_101, "ada-welcomed-late", ADA_HOST);
    let mut bob = Played::new(
        cloud,
        world.sign_up(4_135_056_102, "bob-welcomes", BOB_HOST),
    );
    let (space, channel, _) = acme_with_channel(&world, &ada, &[(&bob.person, Role::Member)]);
    let at = (space.id, channel);
    let package = take_key_package(cloud, &bob.person.token, ada.device);
    let ada_cert = cert_of(cloud, &ada);
    bob.mls.create_group(channel, now()).expect("a group");
    let commit = (bob.mls)
        .add_devices(channel, &[package], &[ada_cert], now())
        .expect("a Commit");
    let welcome = bob.commits(cloud, at, &commit).expect("a Welcome");
    bob.uploads_group_info(cloud, at);
    bob.stores(cloud, at, &welcome, &[ada.device]);
    stale_once(cloud, space.id, 2);

    let ran = received(&ada.machine);

    assert!(
        warnings_naming(&ran, bob.device()).is_empty(),
        "Bob's device is held not admitted: {ran:?}"
    );
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, BOB_HOST], &[]);
}
