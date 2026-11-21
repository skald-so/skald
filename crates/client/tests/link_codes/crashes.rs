//! A run that stops halfway: the new device killed after it took its grant and before the server
//! stored its certificate stores the certificate at its next run; the linked device killed once
//! the certificate is stored still gets the new device into the account's channels at a later run.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::{Cloud, WAIT};
use crate::kit::{assert_members, group, received};
use crate::linking::{NewDevice, STORE_CERT, YES, start_link, typed};
use crate::machine::{DEVICES, NOT_LINKED};
use crate::revocations::account_certs;
use crate::talk::Talk;
use crate::world::{Person, World, run};

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// Whether the server holds a certificate of `device` among `first`'s account's devices.
fn certified(cloud: &Cloud, first: &Person, device: skald_proto::DeviceId) -> bool {
    cloud
        .devices(&first.token)
        .into_iter()
        .any(|held| held.id == device && held.cert.is_some())
}

/// Waits until `done` holds, which it must within [`WAIT`].
#[track_caller]
fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} after {WAIT:?}");
        thread::sleep(Duration::from_millis(50));
    }
}

/// `skald link` on Ada's first machine, typed the new machine's code, confirms it; the relay
/// answers the new machine's request to store its certificate with `503`, and the new machine is
/// killed once it sent one. The server holds no certificate of it then. With the server reachable
/// again, the new machine's next run stores the certificate, which is valid, and it is linked.
#[test]
fn a_new_device_killed_before_it_stores_its_certificate_stores_it_at_its_next_run() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_801_001, "ada-crashes-the-laptop", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);
    cloud.relay.refuse(STORE_CERT, "503 Service Unavailable");

    let mut linking = start_link(&ada, &[]);
    linking.type_line(&typed(&new.code()));
    linking.wait_to_show(LAPTOP);
    linking.type_line(YES);
    until("no request to store the certificate", || {
        cloud.count_sent(new.started, STORE_CERT) >= 1
    });
    new.kill();
    linking.kill();
    assert!(!certified(cloud, &ada, new.device), "a certificate stored");

    cloud.relay.stop_refusing();
    let later = run(&new.machine, &[DEVICES]);
    assert!(later.success, "{later:?}");
    assert!(
        !later.shown().to_lowercase().contains(NOT_LINKED),
        "{later:?}"
    );
    assert!(certified(cloud, &ada, new.device), "no certificate stored");
    let certs = account_certs(cloud, &ada.token);
    assert!(
        skald_crypto::check_valid_device(&ada.master, &certs, &[], new.device).is_ok(),
        "the stored certificate is not valid"
    );
}

/// In a talk of Ada and Bob, Ada's first machine links her new one with its code and is killed
/// once the server holds the new device's certificate. Once the new machine's run ends, it runs
/// again, and Ada's first machine runs: both of the space's channels have Ada's two devices and
/// Bob's.
#[test]
fn a_linked_device_killed_after_the_certificate_adds_the_new_one_at_a_later_run() {
    let talk = Talk::new(
        &[4_175_801_011, 4_175_801_012],
        &["ada-crashes-the-workstation", "bob-waits-for-the-laptop"],
        false,
    );
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let mut new = NewDevice::start(&talk.world, ada, LAPTOP);
    new.open_run(cloud, ada);

    let mut linking = start_link(ada, &[]);
    linking.type_line(&typed(&new.code()));
    linking.wait_to_show(LAPTOP);
    linking.type_line(YES);
    until("no certificate stored", || {
        certified(cloud, ada, new.device)
    });
    linking.kill();
    let shown = new.finish();
    assert!(shown.success, "the new device failed: {}", shown.text);

    let laptop = new.into_person(cloud, ada);
    received(&laptop.machine);
    received(&ada.machine);
    let at = talk.at();
    let devices = [ada.device, talk.played[0].device(), laptop.device];
    for channel in [talk.space.default, at.1] {
        assert_members(&group(cloud, &ada.token, at.0, channel).members, &devices);
    }
}
