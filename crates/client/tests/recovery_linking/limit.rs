//! The devices limit: a device linked with the recovery key whose certificate the server refuses
//! over the limit says so and stops, and a later run on the same device takes it up again.

use crate::fixtures::fronted_with;
use crate::machine::Machine;
use crate::{
    ADA_HOST, AT_THE_LIMIT, LOST, NEW_HOST, STOLEN, STORE_CERT, assert_certified_by_the_master_key,
    certified_now, new_device, recover, recovered, revocations_of, revoked_once_by, sign_up,
};

/// On a server that allows one device with a certificate, Ada's first machine holds it. Her new
/// machine links with the recovery key naming the first device lost: it sends its certificate,
/// which the server refuses, says the account has as many devices as it may and exits failing,
/// with no certificate of it stored and no revocation of the first device signed. A later run on
/// that machine names the first device stolen: its master-key revocation frees the place, and the
/// server stores the new device's certificate.
#[test]
fn over_the_devices_limit_the_new_device_says_so_and_a_later_run_takes_it_up() {
    let (world, _front) = fronted_with(|settings| settings.devices_limit = Some(1));
    let cloud = &world.cloud;
    let ada = sign_up(&world, (4_176_400_001, "ada-recovery-limit"), ADA_HOST);
    let first = ada.person.device;
    let machine = Machine::new().with_host_name(NEW_HOST);
    let since = cloud.relay.mark();

    let refused = recover(
        &world,
        ada.user(),
        &machine,
        &[(LOST, first)],
        Some(&ada.key),
    );

    assert!(!refused.success, "{refused:?}");
    assert!(refused.text.contains(AT_THE_LIMIT), "{refused:?}");
    assert!(
        cloud.count_sent(since, STORE_CERT) >= 1,
        "no certificate sent: {refused:?}"
    );
    assert_eq!(
        certified_now(cloud, &ada.person.token),
        vec![first],
        "{refused:?}"
    );
    let held = revocations_of(cloud, &ada.person, first);
    assert!(held.is_empty(), "the first device revoked: {held:?}");

    recovered(&world, &ada, &machine, &[(STOLEN, first)]);

    let new = new_device(cloud, &ada.person, &[first]);
    revoked_once_by(cloud, &ada.person, first, None);
    assert_certified_by_the_master_key(cloud, &ada.person, new);
}
