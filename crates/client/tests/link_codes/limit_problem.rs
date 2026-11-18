//! The devices limit by its problem: when the server refuses the new device's certificate with the
//! problem `https://skald.so/problems/devices-limit`, the new device fails and says the account
//! `has as many devices as it may`, by the problem's type whatever its `detail`; another refusal,
//! also a `403`, does not say so. The relay answers the request that stores the certificate
//! itself; the server's own limit is tested in `limit.rs` and in `crates/server/tests/cloud_limits`.

use crate::linking::{NewDevice, STORE_CERT, YES, link_with, says, typed};
use crate::terminal::Shown;
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// The server's refusal of a certificate over the limit; its `detail` says otherwise than the CLI.
const OVER_LIMIT: &str = r#"{"type":"https://skald.so/problems/devices-limit","title":"Device limit reached","status":403,"detail":"Refused: 50 of 50."}"#;

/// What the new device says on that refusal.
const SAID: &str = "has as many devices as it may";

/// GitHub user `id` with `login` signs up; a new machine of the account shows its code, and the
/// relay answers its request to store its certificate with `problem`. Linked with the code on the
/// first machine, the new device fails having sent the request, `skald link` fails, and the server
/// holds no certificate of the new device. Gives what the new device showed.
fn refused_with(id: u64, login: &str, problem: &str) -> Shown {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(id, login, "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);
    cloud
        .relay
        .refuse_with(STORE_CERT, "403 Forbidden", problem);

    let linked = link_with(&ada, &typed(&new.code()), LAPTOP, YES);
    let shown = new.finish();

    assert!(!shown.success, "the new device succeeded: {}", shown.text);
    assert!(
        cloud.count_sent(new.started, STORE_CERT) >= 1,
        "no certificate sent"
    );
    assert!(!linked.success, "skald link succeeded: {}", linked.text);
    let certified = cloud
        .devices(&ada.token)
        .into_iter()
        .any(|device| device.id == new.device && device.cert.is_some());
    assert!(!certified, "a refused certificate is stored");
    shown
}

/// A new device whose certificate the server refuses as over the limit says the account has as
/// many devices as it may.
#[test]
fn a_certificate_over_the_limit_says_the_account_has_as_many_devices_as_it_may() {
    let shown = refused_with(4_175_702_001, "ada-many-devices", OVER_LIMIT);
    assert!(
        says(&shown.text, &[SAID]),
        "it does not say the account has as many devices as it may: {}",
        shown.text
    );
}

/// Another refusal of the certificate is not taken for the devices limit.
#[test]
fn another_refusal_of_the_certificate_does_not_say_the_devices_limit() {
    let other_problem = OVER_LIMIT.replace("devices-limit", "not-allowed");
    let shown = refused_with(4_175_702_011, "ada-other-refusal", &other_problem);
    assert!(
        !says(&shown.text, &[SAID]),
        "another refusal says the account has as many devices as it may: {}",
        shown.text
    );
}
