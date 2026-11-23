//! The new device takes a link object only from a `client` device of its account (`membership.md`,
//! "Linking a device"): one that a valid `browser` device of the account signed fails the run.

use skald_proto::DeviceKind;

use crate::fixtures::{Issuer, add_device_of_kind};
use crate::linking::{NewDevice, STORE_CERT};
use crate::played_issuer::{Granted, answer};
use crate::world::World;

/// Ada's account has a valid `browser` device. The played issuing device grants her new device an
/// honest certificate with a link object that the browser signed as its issuer: the run fails at
/// the new device, which sends no request to store its certificate.
#[test]
fn a_link_object_from_a_browser_device_fails_the_run() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_602_001, "ada-gets-a-browsers-link", "ada-workstation");
    let browser = add_device_of_kind(
        cloud,
        (ada.account, DeviceKind::Browser),
        "ada-browser",
        &Issuer::Device(&ada),
    );
    let mut new = NewDevice::start(&world, &ada, "ada-laptop");

    let run = answer(cloud, &ada, &mut new);
    let mut granted = Granted::honest(cloud, &ada, &run);
    granted.link.issuer = browser.id;
    granted.link_signer = browser.signing;
    granted.send(cloud, &ada, &run);
    new.wait_for_failures(1);
    assert_eq!(
        cloud.count_sent(new.started, STORE_CERT),
        0,
        "a certificate sent"
    );
}
