//! The new device takes its certificate and the master key only from a run that passes its checks
//! (`membership.md`, "Link code", the new device's step 3). The test plays the issuing device with
//! Ada's first device's key, from the code the new device shows: a grant that is not for the new
//! device's account, ID, kind, keys and name, whose signature or link object fails, or whose master
//! key is not the one the issuer chains to, fails the run, and the new device stores no
//! certificate. The same played device's honest grant links it.

use skald_crypto::SigningKey;
use skald_proto::{DeviceId, DeviceKind};

use crate::cloud::Cloud;
use crate::linking::{NewDevice, STORE_CERT, THREE_TIMES, says};
use crate::played_issuer::{Granted, answer};
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// A device ID no device of the tests has.
fn stranger() -> DeviceId {
    DeviceId::from_signing_key(&SigningKey::generate().public_key())
}

/// What a refused grant changes in an honest one.
type Change = fn(&mut Granted);

/// The grants the new device must refuse, each by what it changes in an honest one.
fn refused() -> Vec<(&'static str, Change)> {
    vec![
        ("another account", |granted| {
            granted.cert.account = skald_proto::AccountId::generate();
        }),
        ("another device ID", |granted| {
            granted.cert.device = stranger()
        }),
        ("another signing key", |granted| {
            granted.cert.signing = SigningKey::generate().public_key();
        }),
        ("another hpke key", |granted| granted.cert.hpke = [0x42; 32]),
        ("another name", |granted| {
            granted.cert.name = "someone-elses-laptop".to_owned();
        }),
        ("another kind", |granted| {
            granted.cert.kind = DeviceKind::Service
        }),
        ("a signature by another key", |granted| {
            granted.cert_signer = SigningKey::generate();
        }),
        ("another master key", |granted| {
            granted.master = SigningKey::generate().public_key();
        }),
        ("a link object for another device", |granted| {
            granted.link.device = stranger();
        }),
        ("a link object by another key", |granted| {
            granted.link_signer = SigningKey::generate();
        }),
    ]
}

/// The played issuing device sends Ada's new device, run by run, each grant it must refuse, asking
/// it for a fresh code once its code failed 3 times. Each run fails at the new device; it sends no
/// request to store a certificate, and the server holds none of it. Then the played device's
/// honest grant, on a code asked for afresh, links it: it stores a certificate the server holds.
#[test]
fn the_new_device_refuses_a_grant_that_fails_its_checks() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_601_001, "ada-gets-forged-grants", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);

    for (failed, (what, change)) in refused().into_iter().enumerate() {
        if failed > 0 && failed % 3 == 0 {
            new.run().wait_for("that the code failed 3 times", |shown| {
                says(shown, &THREE_TIMES)
            });
            new.fresh_code();
        }
        let run = answer(cloud, &ada, &mut new);
        let mut granted = Granted::honest(cloud, &ada, &run);
        change(&mut granted);
        granted.send(cloud, &ada, &run);
        new.wait_for_failures(failed + 1);
        assert_eq!(
            cloud.count_sent(new.started, STORE_CERT),
            0,
            "{what}: a certificate sent"
        );
    }
    let id = new.device;
    let certified = |cloud: &Cloud| {
        cloud
            .devices(&ada.token)
            .into_iter()
            .any(|device| device.id == id && device.cert.is_some())
    };
    assert!(!certified(cloud), "the server holds a certificate");

    new.fresh_code();
    let run = answer(cloud, &ada, &mut new);
    Granted::honest(cloud, &ada, &run).send(cloud, &ada, &run);
    let shown = new.finish();
    assert!(shown.success, "the honest grant failed: {}", shown.text);
    assert!(certified(cloud), "the server holds no certificate");
}
