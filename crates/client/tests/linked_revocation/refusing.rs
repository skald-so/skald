//! A GroupInfo that also lists a device that is not admitted and not a revoked device of a member
//! account, such as one the server joined or one the roster bars, is refused with a warning, even
//! when its other devices are all revoked; the device joins once no such device is left.

use skald_proto::NonEmpty;

use crate::joins::{forked_group_info, intruder, serve_group_info};
use crate::kit::{
    RECV, assert_listed_not_in, assert_members, channels, latest, received, tell_groups,
    warnings_naming,
};
use crate::linked::{Revoked, THIRD_HOST, revoke_keeping, revoked_group};
use crate::talk::commit_sent;
use crate::world::{ACME, next_with, run, store};

/// Fails unless `refused`, a run of Ada's fourth machine, warns naming `device` and the default
/// channel, sent no Commit in the channel since the front's first `from` sends, and leaves the
/// server's group at its epoch with `held`; and unless `skald channels` on that machine lists the
/// channel as one it is not in.
#[track_caller]
fn assert_refused(
    revoked: &Revoked,
    refused: &crate::machine::Finished,
    (device, from): (skald_proto::DeviceId, usize),
    held: &[skald_proto::DeviceId],
) {
    let channel = revoked.space.default;
    assert!(
        warnings_naming(refused, device)
            .iter()
            .any(|line| line.contains(&channel.to_string())),
        "no warning naming {device} and {channel}: {refused:?}"
    );
    assert!(
        !commit_sent(&revoked.front, channel, from),
        "a Commit sent in the channel: {refused:?}"
    );
    revoked.group().assert_is(revoked.epoch, held);
    assert_listed_not_in(&channels(&revoked.fourth.machine, ACME), channel, &[]);
}

/// The default channel of `acme` holds Ada's first and second machines' devices, both revoked; the
/// relay gives, as its GroupInfo, that of a fork signed by her second machine's device that also
/// lists a device on her account whose certificate chains to another master key, as one the server
/// joined. Her fourth machine's `skald recv` warns naming that device and sends no Commit in the
/// channel. Once the relay gives the server's GroupInfo again, its next run joins and removes the
/// revoked devices: the server's group holds its device alone.
#[test]
fn group_info_also_listing_a_device_the_server_joined_is_refused_with_a_warning() {
    let revoked = revoked_group((4_840_005_001, "ada-meets-an-intruder"), false);
    let cloud = revoked.cloud();
    let (space, channel) = revoked.at();
    let joined = intruder(cloud, &revoked.ada, "ada-server-device");
    let fork = forked_group_info(&revoked.second, channel, &joined);
    serve_group_info(cloud, (space, channel), &fork);
    let from = revoked.front.sent().len();

    let refused = run(&revoked.fourth.machine, &[RECV]);

    let held = [revoked.ada.device, revoked.second.device];
    assert_refused(&revoked, &refused, (joined.id(), from), &held);
    tell_groups(cloud, space, channel);

    received(&revoked.fourth.machine);

    assert_members(&revoked.group().members, &[revoked.fourth.device]);
}

/// The default channel of `acme` holds Ada's first and second machines' devices, both revoked, and
/// her third's, valid; a revision her fourth machine's device signs, as an admin's, bars the third
/// machine's device. Her fourth machine's `skald recv` warns naming the third machine's device and
/// sends no Commit in the channel. Once her fourth machine revokes the third's device, its next run
/// joins and removes the three: the server's group holds its device alone.
#[test]
fn group_info_also_listing_an_excluded_device_is_refused_with_a_warning() {
    let revoked = revoked_group((4_840_006_001, "ada-bars-her-tablet"), true);
    let cloud = revoked.cloud();
    let fourth = &revoked.fourth;
    let third = revoked.third.as_ref().expect("the third machine").device;
    let previous = latest(cloud, &fourth.token, revoked.space.id);
    let barring = next_with(&previous, (fourth.device, &fourth.key), |body| {
        let mut members = body.members.clone().into_vec();
        for member in &mut members {
            if member.account == fourth.account {
                member.excluded = Some(NonEmpty::new(third));
            }
        }
        body.members = NonEmpty::from_vec(members).expect("a member");
    });
    store(cloud, &fourth.token, &barring);
    let from = revoked.front.sent().len();

    let refused = run(&fourth.machine, &[RECV]);

    let held = [revoked.ada.device, revoked.second.device, third];
    assert_refused(&revoked, &refused, (third, from), &held);
    let revoking = revoke_keeping(&fourth.machine, third, &[]);
    assert!(revoking.success, "{THIRD_HOST} not revoked: {revoking:?}");

    received(&fourth.machine);

    assert_members(&revoked.group().members, &[fourth.device]);
}
