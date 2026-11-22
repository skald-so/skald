//! Ada's next run admits Bob, whose acceptance passes the checks: the revision lists him as the
//! invite says, and his device gets into the default channel and the invite's other channel, from
//! Ada's device or by itself, and exchanges messages with Ada's there.

use skald_proto::{ChannelId, Role, SpaceId};

use crate::chain::joins_by_itself;
use crate::fixtures::{person_address, send};
use crate::joins::read_from;
use crate::kit::received;
use crate::played::without_key_packages;
use crate::talk::{Line, encrypted, received_until, stored};
use crate::world::ACME;
use crate::{Scene, assert_told, group_members};

/// Ada's and Bob's devices say these to each other, in the default channel and the other one.
const ADA_SAYS: [&str; 2] = [
    "welcome aboard, the roadmap is pinned",
    "this side channel is for the launch",
];
const BOB_SAYS: [&str; 2] = [
    "thanks, reading the roadmap now",
    "happy to help with the launch",
];

/// Ada's machine sends `ADA_SAYS` in the default channel and the other one, and Bob's device reads
/// each once from Ada's; Bob's device says `BOB_SAYS` there, and Ada's next runs show both.
#[track_caller]
fn assert_talk(scene: &mut Scene) {
    let cloud = &scene.world.cloud;
    let [default, other] = scene.both();
    for ((_, channel), text) in [default, other].into_iter().zip(ADA_SAYS) {
        let said = send(&scene.ada.machine, channel, text);
        assert!(said.success, "{said:?}");
    }
    let bob = &mut scene.bob;
    bob.take_in(cloud);
    for ((_, channel), text) in [default, other].into_iter().zip(ADA_SAYS) {
        assert_eq!(
            read_from(bob, channel, text),
            [scene.ada.device],
            "Bob's device does not read Ada's message in {channel} once"
        );
    }
    let from = person_address(ACME, &bob.person);
    for (at, text) in [default, other].into_iter().zip(BOB_SAYS) {
        let envelope = encrypted(bob, at, at.1, &Line::new(&from, 1, text));
        stored(cloud, bob, &envelope);
    }
    received_until(&scene.ada.machine, |shown| {
        BOB_SAYS.iter().all(|text| shown.contains(text))
    });
}

/// Bob accepts an invite for a member to the default channel, with `history` true, and the other
/// one, with `history` false. Ada's `skald recv` stores one revision, signed by her device, that
/// lists Bob with his handle, the role member, his master key and the invite, in the default
/// channel with `history` true and in the other one with `history` false; it says it admitted Bob
/// to `acme`. Bob's device joins both channels from Welcomes of Ada's device, whose groups then
/// hold Ada's, Carol's and Bob's devices, and exchanges messages with Ada's in both.
#[test]
fn admin_s_next_run_admits_the_newcomer_whose_device_it_adds_to_the_invite_s_channels() {
    let mut scene = Scene::new(
        ACME,
        &[
            (4_855_101_001, "ada-admits-bob"),
            (4_855_101_002, "carol-co-admin"),
            (4_855_101_003, "bob-newcomer"),
        ],
    );
    let made = scene.invite();
    scene.bob_accepts(&made);
    let previous = scene.latest();

    let ran = received(&scene.ada.machine);

    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
    assert_told(&ran, scene.bob.person.handle.as_str(), ACME);
    scene.assert_bob_welcomed();
    let devices = [scene.ada.device, scene.carol.device(), scene.bob.device()];
    for at in scene.both() {
        assert_eq!(
            group_members(scene.cloud(), &scene.ada.token, at),
            devices.into_iter().collect(),
            "the group of {}",
            at.1
        );
    }
    assert_talk(&mut scene);
}

/// Bob accepts an invite for an admin to both channels, with `history` true, while no KeyPackage
/// of his device is on the server. Ada's `skald recv` stores the revision that lists Bob as an
/// admin in both channels with `history` true. Bob's device, which no device added, joins both by
/// itself; Ada's device keeps it in their groups and exchanges messages with it in both.
#[test]
fn newcomer_s_device_with_no_key_package_joins_the_channels_by_itself() {
    let mut scene = Scene::new(
        ACME,
        &[
            (4_855_102_001, "ada-admits-bob-later"),
            (4_855_102_002, "carol-co-admin-later"),
            (4_855_102_003, "bob-no-key-packages"),
        ],
    );
    let other = scene.channel;
    let made = scene.invite_with(|body| {
        body.role = Role::Admin;
        body.channels = vec![(body.channels[0].0, true), (other, true)];
    });
    scene.bob_accepts(&made);
    let cloud = &scene.world.cloud;
    without_key_packages(cloud, scene.bob.device());
    let previous = scene.latest();

    received(&scene.ada.machine);

    scene.assert_bob_admitted(&previous, scene.ada.device, &made);
    let both: [(SpaceId, ChannelId); 2] = scene.both();
    for at in both {
        joins_by_itself(cloud, &mut scene.bob, at);
    }
    received(&scene.ada.machine);
    let devices = [scene.ada.device, scene.carol.device(), scene.bob.device()];
    for at in both {
        assert_eq!(
            group_members(cloud, &scene.ada.token, at),
            devices.into_iter().collect(),
            "the group of {}",
            at.1
        );
    }
    assert_talk(&mut scene);
}
