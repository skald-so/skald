//! Revoking a linked device: the command shows the devices it certified; those its user names stay
//! valid at the account's devices and at other members' and stay in the channels, while the
//! revoked device is removed; without `keep`, they are revoked and removed too.

use skald_crypto::DeviceCheckError;
use skald_proto::{DeviceId, Revocation};

use crate::devices::second_machine;
use crate::fixtures::send;
use crate::joins::read_from;
use crate::kit::{assert_members, group, received};
use crate::linked::{SECOND_HOST, THIRD_HOST, checked_by_member, devices_of, revoked_keeping};
use crate::revocations::{account_revocations, assert_listed, assert_removed_carrying, naming};
use crate::talk::Talk;
use crate::world::Person;

/// The text Ada's third machine sends once her second is revoked.
const STILL_HERE: &str = "the tablet still files the reports";

/// Ada and Bob in a channel of `acme`, as [`Talk::new`] sets them up; Ada's second machine,
/// linked by her first, and her third, linked by her second, each join both channels of `acme` by
/// themselves, and Bob's device and Ada's first machine take in their joins. Gives Ada's second
/// and third machines.
#[track_caller]
fn chain_of_three(ids: [u64; 2], logins: [&str; 2]) -> (Talk, Person, Person) {
    let mut talk = Talk::new(&ids, &logins, false);
    let second = second_machine(&talk.world, &talk.ada, SECOND_HOST);
    received(&second.machine);
    let third = second_machine(&talk.world, &second, THIRD_HOST);
    received(&third.machine);
    let cloud = &talk.world.cloud;
    talk.played[0].take_in(cloud);
    received(&talk.ada.machine);
    let devices = [
        talk.ada.device,
        talk.played[0].device(),
        second.device,
        third.device,
    ];
    for channel in [talk.space.default, talk.channel] {
        assert_members(
            &group(cloud, &talk.ada.token, talk.space.id, channel).members,
            &devices,
        );
        assert_members(&talk.played[0].members(channel), &devices);
    }
    (talk, second, third)
}

/// The one revocation of `device` the server holds for Ada's account; fails unless there is one,
/// signed by her first machine's device.
#[track_caller]
fn one_revocation_of(talk: &Talk, device: DeviceId) -> Revocation {
    let held = naming(&account_revocations(talk.cloud(), &talk.ada.token), device);
    assert_eq!(held.len(), 1, "not one revocation of {device}: {held:?}");
    assert_eq!(
        held[0].body().signer,
        Some(talk.ada.device),
        "not signed by Ada's first machine's device: {held:?}"
    );
    held[0].clone()
}

/// Fails unless both channels of the talk's space hold `devices` and no other, in the server's
/// GroupInfo and at Bob's device once it took in its mailbox, and Bob's device entered a Commit of
/// Ada's first machine's device in each that removed `removed` carrying `revocation`.
#[track_caller]
fn assert_channels_hold(
    talk: &mut Talk,
    devices: &[DeviceId],
    (removed, revocation): (DeviceId, &Revocation),
) {
    let (cloud, ada) = (&talk.world.cloud, talk.ada.device);
    let bob = &mut talk.played[0];
    bob.take_in(cloud);
    for channel in [talk.space.default, talk.channel] {
        assert_members(
            &group(cloud, &talk.ada.token, talk.space.id, channel).members,
            devices,
        );
        assert_members(&bob.members(channel), devices);
        assert_removed_carrying(bob, channel, (ada, removed), revocation);
    }
}

/// Ada's second machine was linked by her first, and her third by her second. Her first machine
/// revokes her second's device keeping her third's: the command shows the third machine. The
/// server holds one revocation of the second machine's device, signed by the first's, whose `keep`
/// lists the third's. Ada's `skald devices` lists the second revoked and the third not; as Bob's
/// session gets the space's devices, the third is valid and the second revoked. Ada's first
/// machine removes the second from both channels by Commits carrying the revocation, which Bob's
/// device enters, and the third stays in both: it sends in the channel, and Bob's device reads it.
#[test]
fn revocation_keeps_the_certified_devices_its_user_names() {
    let (mut talk, second, third) = chain_of_three(
        [4_840_001_001, 4_840_001_002],
        ["ada-keeps-her-tablet", "bob-sees-the-tablet"],
    );

    revoked_keeping(
        &talk.ada.machine,
        second.device,
        &[third.device],
        &[THIRD_HOST],
    );
    received(&talk.ada.machine);

    let revocation = one_revocation_of(&talk, second.device);
    let kept: Option<Vec<DeviceId>> =
        (revocation.body().keep.as_ref()).map(|keep| keep.iter().copied().collect());
    assert_eq!(kept, Some(vec![third.device]), "{:?}", revocation.body());
    let shown = devices_of(&talk.ada.machine);
    assert_listed(&shown, SECOND_HOST, true);
    assert_listed(&shown, THIRD_HOST, false);
    let bob = (talk.played[0].person.token.as_str(), talk.space.id);
    let master = talk.ada.master;
    assert!(
        checked_by_member(talk.cloud(), bob, &master, third.device).is_ok(),
        "the kept device is not valid for Bob"
    );
    assert!(
        matches!(
            checked_by_member(talk.cloud(), bob, &master, second.device),
            Err(DeviceCheckError::Revoked { .. })
        ),
        "the revoked device is not revoked for Bob"
    );
    let devices = [talk.ada.device, talk.played[0].device(), third.device];
    assert_channels_hold(&mut talk, &devices, (second.device, &revocation));
    let said = send(&third.machine, talk.channel, STILL_HERE);
    assert!(said.success, "{said:?}");
    let bob = &mut talk.played[0];
    bob.take_in(&talk.world.cloud);
    assert_eq!(
        read_from(bob, talk.channel, STILL_HERE),
        [third.device],
        "Bob's device does not read the kept device's message once"
    );
}

/// As above, Ada's first machine revokes her second's device naming none to keep: the command
/// shows her third machine, as certified by the second. The server holds one revocation of the
/// second machine's device, signed by the first's, without `keep`. Ada's `skald devices` lists the
/// second and the third revoked; as Bob's session gets the space's devices, both are revoked. Ada's
/// first machine removes both from both channels, by Commits carrying the revocation, which Bob's
/// device enters.
#[test]
fn revocation_without_keep_revokes_and_removes_the_devices_the_revoked_one_certified() {
    let (mut talk, second, third) = chain_of_three(
        [4_840_002_001, 4_840_002_002],
        ["ada-keeps-nothing", "bob-loses-the-tablet"],
    );

    revoked_keeping(&talk.ada.machine, second.device, &[], &[THIRD_HOST]);
    received(&talk.ada.machine);

    let revocation = one_revocation_of(&talk, second.device);
    assert_eq!(revocation.body().keep, None, "{:?}", revocation.body());
    let shown = devices_of(&talk.ada.machine);
    assert_listed(&shown, SECOND_HOST, true);
    assert_listed(&shown, THIRD_HOST, true);
    let bob = (talk.played[0].person.token.as_str(), talk.space.id);
    let master = talk.ada.master;
    for device in [second.device, third.device] {
        assert!(
            matches!(
                checked_by_member(talk.cloud(), bob, &master, device),
                Err(DeviceCheckError::Revoked { .. })
            ),
            "{device} is not revoked for Bob"
        );
    }
    let devices = [talk.ada.device, talk.played[0].device()];
    assert_channels_hold(&mut talk, &devices, (second.device, &revocation));
    let bob = &talk.played[0];
    let removed_third = bob.changes_of(talk.channel).iter().any(|change| {
        change.sender == talk.ada.device
            && change
                .removed
                .iter()
                .any(|member| member.device == third.device)
    });
    assert!(
        removed_third,
        "Ada's first machine did not remove the third"
    );
}
