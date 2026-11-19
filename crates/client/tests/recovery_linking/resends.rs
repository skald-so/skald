//! Revocations a device linked with the recovery key signed whose answers were lost: the next run
//! that connects sends them again, with the same bytes, and a later run with the recovery key on
//! the same device finishes what a killed run left, with the certificate it stored.

use skald_proto::DeviceId;

use crate::linked::devices_of;
use crate::machine::Machine;
use crate::relay::Lose;
use crate::revocations::STORE_REVOCATION;
use crate::terminal::Shown;
use crate::world::World;
use crate::{
    ADA_HOST, Ada, LOST, NEW_HOST, STOLEN, assert_certified_by_the_master_key, cert_of, new_device,
    played, recovered, revocations_of, revoked_once_by, sign_up,
};

/// The relay withholds the server's answer to the next request whose line starts with `request`,
/// while Ada links `machine` with her recovery key naming `named`, and the run is killed once
/// it is withheld; gives what it showed.
#[track_caller]
fn killed_at(
    world: &World,
    ada: &Ada,
    machine: &Machine,
    (named, request): (&[(&str, DeviceId)], &str),
) -> Shown {
    let cloud = &world.cloud;
    cloud.relay.lose_answer(request, Lose::Withhold);
    let losing = || cloud.relay.lost() > 0;
    let key = (ada.user(), Some(ada.key.as_str()));
    let killed = played(world, key, machine, named, losing).kill();
    assert_eq!(cloud.relay.lost(), 1, "no answer lost: {killed:?}");
    killed
}

/// The relay withholds the server's answer to the master-key revocation of Ada's stolen first
/// device, and the run is killed. A later run with the recovery key on the same machine, then
/// `skald devices`, leave the server one revocation of the first device, by the master key, and the
/// new device's certificate, valid under the master key.
#[test]
fn a_killed_run_is_finished_by_a_later_one_without_a_second_master_key_revocation() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_700_001, "ada-recovery-killed"), ADA_HOST);
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let named = [(STOLEN, first)];

    killed_at(&world, &ada, &machine, (&named, STORE_REVOCATION));
    recovered(&world, &ada, &machine, &named);
    devices_of(&machine);

    revoked_once_by(cloud, &ada.person, first, None);
    let new = new_device(cloud, &ada.person, &[first]);
    assert_certified_by_the_master_key(cloud, &ada.person, new);
}

/// The relay loses the server's answer to the new device's revocation of Ada's lost first device,
/// closing the connection, or withholding it while the run is killed. `skald devices` on the new
/// machine sends it again: the server holds one revocation of the first device, by the new one.
#[test]
fn a_lost_answer_to_the_new_devices_revocation_is_recovered_by_the_next_run() {
    for (how, user) in [
        (Lose::Close, (4_176_700_101, "ada-recovery-closed")),
        (Lose::Withhold, (4_176_700_201, "ada-recovery-withheld")),
    ] {
        let world = World::start();
        let cloud = &world.cloud;
        let ada = sign_up(&world, user, ADA_HOST);
        let first = ada.person.device;
        let machine = Machine::new().with_host_name(NEW_HOST);
        let since = cloud.relay.mark();
        cloud.relay.lose_answer(STORE_REVOCATION, how);

        let losing = || how == Lose::Withhold && cloud.relay.lost() > 0;
        let key = (ada.user(), Some(ada.key.as_str()));
        let ended = played(&world, key, &machine, &[(LOST, first)], losing).kill();
        assert_eq!(cloud.relay.lost(), 1, "{how:?}: no answer lost: {ended:?}");
        devices_of(&machine);

        let new = new_device(cloud, &ada.person, &[first]);
        revoked_once_by(cloud, &ada.person, first, Some(new));
        let sent = cloud.count_sent(since, STORE_REVOCATION);
        assert!(sent >= 2, "{how:?}: sent {sent} times: {ended:?}");
    }
}

/// The request whose answer gives the account's anchors objects, as its line starts.
const GIVE_ANCHORS: &str = "GET /account/anchors ";

/// The relay withholds the server's answer to the anchors request of Ada's new machine, after the
/// server stored its certificate and before it revokes her lost first device, and the run is
/// killed. A later run with the recovery key on the same machine finishes with that certificate:
/// the server holds it still, and a revocation of the first device by the new one.
#[test]
fn a_run_killed_after_its_certificate_is_stored_is_finished_with_that_certificate() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_700_301, "ada-recovery-stored"), ADA_HOST);
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let named = [(LOST, first)];

    let killed = killed_at(&world, &ada, &machine, (&named, GIVE_ANCHORS));
    let new = new_device(cloud, &ada.person, &[first]);
    let stored = cert_of(cloud, &ada.person, new).expect("the certificate stored");
    assert!(
        revocations_of(cloud, &ada.person, first).is_empty(),
        "the first device revoked: {killed:?}"
    );

    recovered(&world, &ada, &machine, &named);

    assert_eq!(cert_of(cloud, &ada.person, new), Some(stored));
    assert_certified_by_the_master_key(cloud, &ada.person, new);
    revoked_once_by(cloud, &ada.person, first, Some(new));
}
