//! Forming a channel's group: the first admitted device to accept the revision with the channel
//! creates it with every admitted device it gets a valid KeyPackage for; of two devices that create
//! it at once, both end in the group the server kept; a Welcome from a device that is not admitted
//! forms no group.

use skald_crypto::Joining;
use skald_proto::{ChannelId, DeviceId, Frame, Role};

use crate::cloud::Cloud;
use crate::fixtures::fronted;
use crate::front::Front;
use crate::kit::{
    EPOCH_MISMATCH, Space, assert_listed_not_in, assert_listed_with, assert_members, channels,
    create_space, group, group_on_server, hide_group, received, swap_key_packages, tell_groups,
    warnings_naming, with_channel,
};
use crate::played::{Played, mls_envelope, stranger, take_key_package, welcome_from};
use crate::world::{ACME, Person, World, now};

/// The host names of the people's machines, their certificates' `name`.
pub const ADA_HOST: &str = "ada-workstation";
pub const BOB_HOST: &str = "bob-workstation";
pub const CAROL_HOST: &str = "carol-laptop";

/// Ada, whose machine runs the CLI and created `acme`, Bob, whose device the test plays, and a
/// channel of `acme` with both, which revision 3, a fixture Ada's device signs, adds together with
/// Bob, who joins the default channel too. Ada's machine has not accepted revision 3 yet.
pub struct Team {
    pub world: World,
    pub front: Front,
    pub ada: Person,
    pub bob: Played,
    pub space: Space,
    pub channel: ChannelId,
}

impl Team {
    /// The team, Ada's machine retrying for the tests' timeout if `retrying`.
    pub fn new(ids: [u64; 2], logins: [&str; 2], retrying: bool) -> Self {
        let (world, front) = fronted();
        let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
        let ada = if retrying {
            crate::fixtures::retrying_for_the_timeout(ada)
        } else {
            ada
        };
        let bob = Played::new(&world.cloud, world.sign_up(ids[1], logins[1], BOB_HOST));
        let cloud = &world.cloud;
        let space = create_space(cloud, &ada, ACME);
        let channel = with_channel(cloud, space.id, &ada, &[(&bob.person, Role::Member)]);
        Team {
            world,
            front,
            ada,
            bob,
            space,
            channel,
        }
    }

    pub fn cloud(&self) -> &Cloud {
        &self.world.cloud
    }

    /// The channel's space and ID.
    pub fn at(&self) -> (skald_proto::SpaceId, ChannelId) {
        (self.space.id, self.channel)
    }

    /// Ada's machine goes through its mailbox and brings the groups in line, then Bob's device
    /// takes in its mailbox.
    pub fn form(&mut self) {
        received(&self.ada.machine);
        let cloud = &self.world.cloud;
        self.bob.take_in(cloud);
    }
}

/// Fails unless `joined` holds one Welcome of `channel` at `epoch`, whose members are `devices`.
#[track_caller]
pub fn assert_one_welcome(
    joined: &[Joining],
    channel: ChannelId,
    epoch: u64,
    devices: &[DeviceId],
) {
    assert_eq!(joined.len(), 1, "not one Welcome of {channel}: {joined:?}");
    assert_eq!(joined[0].epoch, epoch, "the Welcome's epoch");
    let members: Vec<DeviceId> = joined[0]
        .members
        .iter()
        .map(|member| member.device)
        .collect();
    assert_members(&members, devices);
}

/// The channel, which revision 3 adds with Ada and Bob, has no group yet: Ada's device creates it
/// with Bob's device at epoch 1, uploads its GroupInfo and sends Bob's device the Welcome. Each
/// device lists both.
#[test]
fn channel_ada_s_device_creates_is_at_epoch_1_with_both_devices_and_each_lists_both() {
    let mut team = Team::new(
        [4_135_052_001, 4_135_052_002],
        ["ada-creates-group", "bob-in-group"],
        false,
    );
    let (ada, bob_device) = (team.ada.device, team.bob.device());

    team.form();

    let cloud = team.cloud();
    let held = group(cloud, &team.ada.token, team.space.id, team.channel);
    held.assert_is(1, &[ada, bob_device]);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
    let joined = team.bob.joined_to(team.channel);
    assert_one_welcome(&joined, team.channel, 1, &[ada, bob_device]);
    assert_eq!(joined[0].signer.device, ada, "the Welcome's signer");
    assert_members(&team.bob.members(team.channel), &[ada, bob_device]);
    let listed = channels(&team.ada.machine, ACME);
    assert_listed_with(&listed, team.channel, &[ADA_HOST, BOB_HOST], &[]);
}

/// Bob's device creates the channel's group with Ada's device first, while Ada's device, told by a
/// proxy that the channel has no group yet, creates one too: its Commit gets `epoch-mismatch`.
/// Once Bob's device sends its Welcome, Ada's device joins the kept group; as its member it then
/// uploads the GroupInfo of the epoch Bob's next Commit starts, which the server had none of.
#[test]
fn devices_creating_one_channel_s_group_at_once_end_in_the_kept_one() {
    let mut team = Team::new(
        [4_135_052_101, 4_135_052_102],
        ["ada-creates-too", "bob-creates-first"],
        false,
    );
    let (ada, bob_device) = (team.ada.device, team.bob.device());
    let at = team.at();
    let cloud = &team.world.cloud;
    let ada_package = take_key_package(cloud, &team.bob.person.token, ada);
    let ada_cert = crate::fixtures::cert_of(cloud, &team.ada);
    team.bob
        .mls
        .create_group(team.channel, now())
        .expect("a group");
    let commit = (team.bob.mls)
        .add_devices(team.channel, &[ada_package], &[ada_cert], now())
        .expect("a Commit");
    let welcome = team.bob.commits(cloud, at, &commit).expect("a Welcome");
    hide_group(cloud, team.space.id, team.channel);

    // The run may end without the group: it has no Welcome yet.
    let _ = crate::world::run(&team.ada.machine, &[crate::kit::RECV]);

    assert!(
        team.front.answered().iter().any(|answer| matches!(
            answer,
            Frame::SendRefused { problem, .. } if problem == EPOCH_MISMATCH
        )),
        "Ada's device sent no Commit of its own group: {:?}",
        team.front.answered()
    );
    tell_groups(cloud, team.space.id, team.channel);
    team.bob.uploads_group_info(cloud, at);
    team.bob.stores(cloud, at, &welcome, &[ada]);

    received(&team.ada.machine);

    let listed = channels(&team.ada.machine, ACME);
    assert_listed_with(&listed, team.channel, &[ADA_HOST, BOB_HOST], &[]);
    let empty = (team.bob.mls)
        .commit_empty(team.channel, b"", now())
        .expect("an empty Commit");
    team.bob.commits(cloud, at, &empty);
    assert!(
        group_on_server(cloud, &team.ada.token, at.0, at.1)
            .1
            .is_none(),
        "the server has a GroupInfo of the new epoch already"
    );

    received(&team.ada.machine);

    let held = group(cloud, &team.ada.token, at.0, at.1);
    held.assert_is(2, &[ada, bob_device]);
    assert_eq!(held.signer, ada, "Ada's device uploaded no GroupInfo");
}

/// A device that is no account's, and so not admitted, forms a group with the channel's ID and puts
/// its Welcome to Ada's device into Ada's catch-up, as a compromised server would; the channel's
/// own group, which Bob's device created, has no GroupInfo yet. Ada's device stays without the
/// group, warns naming that device, and lists the channel as one it is admitted to but not in.
#[test]
fn welcome_from_a_device_not_admitted_leaves_the_device_without_the_group_with_a_warning() {
    let mut team = Team::new(
        [4_135_052_201, 4_135_052_202],
        ["ada-refuses-welcome", "bob-alone-first"],
        false,
    );
    let at = team.at();
    let cloud = &team.world.cloud;
    team.bob.creates_alone(cloud, at);
    let mut eve = stranger();
    let welcome = welcome_from(
        cloud,
        &mut eve,
        team.channel,
        (&team.ada, &team.bob.person.token),
    );
    team.front.inject(
        team.ada.device,
        vec![mls_envelope(at, &welcome, &[team.ada.device])],
    );

    let ran = received(&team.ada.machine);

    assert_eq!(team.front.injected(), 1, "not put into catch-up: {ran:?}");
    assert!(
        !warnings_naming(&ran, eve.id()).is_empty(),
        "no warning naming the Welcome's signer: {ran:?}"
    );
    let listed = channels(&team.ada.machine, ACME);
    assert_listed_not_in(&listed, team.channel, &[ADA_HOST]);
    assert!(
        group_on_server(cloud, &team.ada.token, at.0, at.1)
            .1
            .is_none(),
        "a GroupInfo of the channel"
    );
}

/// Ada, Bob and Carol in a channel that revision 3 adds; a proxy gives `swapped(team)` in place of
/// every KeyPackage of Carol's device the server hands out. Ada's device forms the group with Bob's
/// device alone, each once, and Bob's device joins it.
fn swapped_key_package_is_not_added(
    ids: [u64; 3],
    logins: [&str; 3],
    swapped: impl FnOnce(&mut Played, &Played) -> Vec<u8>,
) {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(ids[0], logins[0], ADA_HOST);
    let mut bob = Played::new(cloud, world.sign_up(ids[1], logins[1], BOB_HOST));
    let carol = Played::new(cloud, world.sign_up(ids[2], logins[2], CAROL_HOST));
    let space = create_space(cloud, &ada, ACME);
    let added = [(&bob.person, Role::Member), (&carol.person, Role::Member)];
    let channel = with_channel(cloud, space.id, &ada, &added);
    let package = swapped(&mut bob, &carol);
    swap_key_packages(cloud, carol.device(), &package);

    received(&ada.machine);

    let (ada_device, bob_device) = (ada.device, bob.device());
    group(cloud, &ada.token, space.id, channel).assert_is(1, &[ada_device, bob_device]);
    bob.take_in(cloud);
    assert_one_welcome(
        &bob.joined_to(channel),
        channel,
        1,
        &[ada_device, bob_device],
    );
    let listed = channels(&ada.machine, ACME);
    assert_listed_with(&listed, channel, &[ADA_HOST, BOB_HOST], &[CAROL_HOST]);
}

/// The KeyPackage given as Carol's is one of a device that is no account's, which is not admitted.
#[test]
fn key_package_swapped_for_one_of_a_device_not_admitted_is_not_added() {
    swapped_key_package_is_not_added(
        [4_135_052_301, 4_135_052_302, 4_135_052_303],
        [
            "ada-swap-stranger",
            "bob-swap-stranger",
            "carol-swap-stranger",
        ],
        |_, _| {
            let package = stranger().key_package(now()).expect("a KeyPackage");
            package.as_ref().to_vec()
        },
    );
}

/// The KeyPackage given as Carol's is a valid one of Bob's device: its `identity` is another
/// device's than the one asked for.
#[test]
fn key_package_swapped_for_one_with_another_identity_is_not_added() {
    swapped_key_package_is_not_added(
        [4_135_052_401, 4_135_052_402, 4_135_052_403],
        [
            "ada-swap-identity",
            "bob-swap-identity",
            "carol-swap-identity",
        ],
        |bob, _| {
            let package = bob.mls.key_package(now()).expect("a KeyPackage");
            package.as_ref().to_vec()
        },
    );
}
