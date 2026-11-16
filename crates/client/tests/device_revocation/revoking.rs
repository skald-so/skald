//! The revocation command: on a device with a valid certificate, it signs a revocation of a device
//! of its account that has one, without `keep`, and has the server store it, resending it until
//! the server has taken it; `skald devices` then shows the device revoked.

use skald_crypto::{DeviceCheckError, SigningKey, check_valid_device};
use skald_proto::DeviceId;

use crate::fixtures::{Issuer, add_device, fronted};
use crate::kit::RECV;
use crate::machine::{DEVICES, FIRST_DEVICE, Machine, NO};
use crate::relay::Lose;
use crate::revocations::{
    STORE_REVOCATION, account_certs, account_revocations, assert_listed, revoke_device,
    revoked_by_cli,
};
use crate::talk::ADA_HOST;
use crate::world::{Person, run};

/// The certificate `name` of the other device of Ada's account the tests write to the server's
/// database.
const OLD_LAPTOP: &str = "ada-old-laptop";

/// Runs `skald devices` on `machine`; fails unless it succeeds.
#[track_caller]
fn devices(machine: &Machine) -> crate::machine::Finished {
    let shown = run(machine, &[DEVICES]);
    assert!(shown.success, "{shown:?}");
    shown
}

/// Fails unless the server holds one revocation of `ada`'s account, which names `device`, signed by
/// her machine's device without `keep` or `also`, and which revokes `device` and not her machine's
/// device.
#[track_caller]
fn assert_stored_once(cloud: &crate::cloud::Cloud, ada: &Person, device: DeviceId) {
    let stored = account_revocations(cloud, &ada.token);
    assert_eq!(stored.len(), 1, "not one revocation stored: {stored:?}");
    let body = stored[0].body();
    assert_eq!(
        (body.account, body.device, body.signer),
        (ada.account, device, Some(ada.device)),
        "the revocation: {body:?}"
    );
    assert_eq!((&body.keep, &body.also), (&None, &None), "{body:?}");
    let certs = account_certs(cloud, &ada.token);
    assert!(
        matches!(
            check_valid_device(&ada.master, &certs, &stored, device),
            Err(DeviceCheckError::Revoked { device: revoked }) if revoked == device
        ),
        "the revocation does not revoke {device}"
    );
    assert!(
        check_valid_device(&ada.master, &certs, &stored, ada.device).is_ok(),
        "the revocation revokes Ada's machine's device"
    );
}

/// Ada's machine revokes another device of her account, which her machine's device certified. The
/// server holds the revocation, signed by her machine's device without `keep`; `skald devices`
/// shows that device revoked, and her machine's device not.
#[test]
fn device_revokes_another_device_of_its_account_which_the_device_list_shows_revoked() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_139_091_001, "ada-revokes-laptop", ADA_HOST);
    let old = add_device(cloud, ada.account, OLD_LAPTOP, &Issuer::Device(&ada));
    assert_listed(&devices(&ada.machine), OLD_LAPTOP, false);

    revoked_by_cli(&ada.machine, old.id);

    assert_stored_once(cloud, &ada, old.id);
    let shown = devices(&ada.machine);
    assert_listed(&shown, OLD_LAPTOP, true);
    assert_listed(&shown, ADA_HOST, false);
}

/// The relay refuses the revocation before the server gets it; the next `skald recv` sends it
/// again, and the relay loses the server's answer; the run after that sends it once more. The
/// server holds the revocation once, and `skald devices` shows the device revoked.
#[test]
fn revocation_the_server_has_not_taken_is_resent_and_stored_once() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_139_091_101, "ada-revokes-again", ADA_HOST);
    let old = add_device(cloud, ada.account, OLD_LAPTOP, &Issuer::Device(&ada));
    cloud
        .relay
        .refuse(STORE_REVOCATION, "503 Service Unavailable");

    // The run may fail, or say the revocation waits, as the server never gets it.
    let _ = revoke_device(&ada.machine, old.id);

    assert!(
        account_revocations(cloud, &ada.token).is_empty(),
        "a revocation stored"
    );
    cloud.relay.stop_refusing();
    cloud.relay.lose_answer(STORE_REVOCATION, Lose::Close);
    let mark = cloud.relay.mark();
    // The run may fail, as its answer is lost.
    let _ = run(&ada.machine, &[RECV]);
    assert_eq!(cloud.relay.lost(), 1, "the revocation was not sent again");
    let _ = run(&ada.machine, &[RECV]);

    assert!(
        cloud.count_sent(mark, STORE_REVOCATION) >= 2,
        "the revocation was not sent again after its answer was lost"
    );
    assert_stored_once(cloud, &ada, old.id);
    assert_listed(&devices(&ada.machine), OLD_LAPTOP, true);
}

/// The command refuses and sends no revocation: on Ada's machine, for Bob's device, for a device of
/// her account without a certificate and for a device the server does not list; on a machine of
/// her account without a valid certificate, for her first machine's device. Ada's machine's device
/// stays valid, and then revokes another device of her account, the one revocation sent.
#[test]
fn devices_of_other_accounts_or_without_a_certificate_are_not_revoked_nor_by_one() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_139_091_201, "ada-revokes-wrongly", ADA_HOST);
    let bob = world.sign_up(4_139_091_202, "bob-not-hers", "bob-workstation");
    let uncertified = add_device(cloud, ada.account, "ada-unlinked", &Issuer::Nobody);
    let unknown = DeviceId::from_signing_key(&SigningKey::generate().public_key());
    let second = Machine::new().with_host_name("ada-second-desk");
    // The machine signs in and waits for linking.
    let _ = cloud.signed_in(
        &world.browser.page(),
        &second,
        &ada.user,
        &[FIRST_DEVICE, NO],
    );
    let mark = cloud.relay.mark();

    for device in [bob.device, uncertified.id, unknown] {
        let refused = revoke_device(&ada.machine, device);
        assert!(!refused.success, "{device} revoked: {refused:?}");
    }
    let refused = revoke_device(&second, ada.device);

    assert!(
        !refused.success,
        "revoked without a certificate: {refused:?}"
    );
    assert_eq!(
        cloud.count_sent(mark, STORE_REVOCATION),
        0,
        "a revocation sent"
    );
    assert!(
        account_revocations(cloud, &ada.token).is_empty(),
        "a revocation stored"
    );
    assert!(
        account_revocations(cloud, &bob.token).is_empty(),
        "Bob's device revoked"
    );
    assert_listed(&devices(&ada.machine), ADA_HOST, false);
    let old = add_device(cloud, ada.account, OLD_LAPTOP, &Issuer::Device(&ada));
    revoked_by_cli(&ada.machine, old.id);
    assert_eq!(cloud.count_sent(mark, STORE_REVOCATION), 1, "not one sent");
    assert_stored_once(cloud, &ada, old.id);
}
