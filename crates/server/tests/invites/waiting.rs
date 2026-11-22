//! At an acceptance, a `space-notice` to the devices of the space's admins, on every instance, and
//! the waiting acceptance, with its account's devices and revocations, to admins' sessions only.

use skald_proto::DeviceKind;

use crate::common::devices::{DEVICES, assert_taken, devices_of};
use crate::link::WAIT;
use crate::scene::{Scene, Waiting, acceptance, gets_no_waiting};
use crate::watch::Watch;

/// eve's acceptance, stored through the first instance, sends a notice of `acme` to alice's laptop
/// there and to bob's on a second instance, the space's admins, and none to carol's and dave's,
/// members who are no admins.
#[test]
fn at_an_acceptance_the_admins_devices_get_a_space_notice_on_any_instance_and_others_none() {
    let scene = Scene::new(3_817_400_001);
    let t = scene.team();
    let flow = &t.flow;
    let first = &flow.stand.server;
    let other = flow.stand.instance(&[], &[]);
    let invite = scene.invite();
    let mut admins = [
        Watch::from(first, flow, &t.alice, 0, &[]),
        Watch::from(&other, flow, &t.bob, 0, &[]),
    ];
    let mut members = [
        Watch::from(first, flow, &t.carol, 0, &[]),
        Watch::from(first, flow, &t.dave, 0, &[]),
    ];

    let eve = &scene.f.eve;
    assert_taken(&eve.session.accept(invite.id, &acceptance(&invite, eve)));

    for watch in &mut admins {
        assert_eq!(watch.notice_within(WAIT), scene.space());
    }
    for watch in &mut members {
        watch.assert_quiet();
    }
}

/// With eve's tablet revoked, alice's and bob's sessions get eve's waiting acceptance with eve's
/// devices, each with its certificate or none, and eve's revocation, as eve's own devices list
/// gives them; carol's and dave's, members who are no admins, and eve's and frank's, outside the
/// space, get none.
#[test]
fn admins_sessions_get_the_waiting_acceptance_with_its_accounts_devices_and_revocations() {
    let scene = Scene::new(3_817_400_101);
    let t = scene.team();
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let tablet = eve.certified_by(scene.f.db(), &eve.laptop, DeviceKind::Client, "Tablet");
    eve.revoke(&eve.by_device(&eve.laptop, tablet.id));
    let invite = scene.invite();
    let accepted = acceptance(&invite, eve);
    assert_taken(&eve.session.accept(invite.id, &accepted));

    let expected = Waiting {
        acceptance: accepted.clone(),
        devices: devices_of(&eve.session.call("get", DEVICES, None)),
        revocations: eve.session.revocations(),
    };
    assert_eq!(expected.devices.len(), 3, "eve's devices: {expected:?}");
    assert_eq!(expected.revocations.len(), 1, "eve's revocation");
    for admin in [&t.alice, &t.bob] {
        let what = format!("{}'s", admin.handle);
        assert_eq!(
            admin.session.waiting(scene.space()),
            vec![expected.clone()],
            "{what}"
        );
    }
    for person in [&t.carol, &t.dave, eve, frank] {
        let what = format!("{}'s", person.handle);
        gets_no_waiting(&person.session, scene.space(), &[&accepted], &what);
    }
}
