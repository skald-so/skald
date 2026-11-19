//! The devices limit at linking: a new device registers at the limit, and the server refuses its
//! certificate. The new device then says the account has as many devices as it may, and the
//! linked device's `skald link` fails; once a device of the account is revoked, the new device's
//! next run stores the certificate it was granted.

use skald_server::Settings;

use crate::devices::second_machine;
use crate::fixtures::fronted_with;
use crate::linking::{NewDevice, STORE_CERT, YES, link_with, says, typed};
use crate::revocations::revoked_by_cli;
use crate::world::run;

/// The host names of Ada's machine linked by hand and of the one linked with a code.
const DESKTOP: &str = "ada-desktop";
const LAPTOP: &str = "ada-laptop";

/// What the new device says on the refusal.
const SAID: &str = "has as many devices as it may";

/// Under a limit of 2 devices, Ada's account has 2 certified: her first machine and one linked by
/// hand. Her new machine signs in at the limit and shows a code; linked with it on her first
/// machine, it is refused its certificate: it says the account has as many devices as it may and
/// fails, `skald link` fails, and the server holds no certificate of it. Her first machine revokes
/// the one linked by hand; the new machine's next run stores its certificate.
#[test]
fn a_certificate_over_the_devices_limit_is_stored_once_a_place_is_free() {
    let (world, _front) = fronted_with(|settings: &mut Settings| settings.devices_limit = Some(2));
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_701_001, "ada-at-the-limit", "ada-workstation");
    let desktop = second_machine(&world, &ada, DESKTOP);
    let certified = |device| {
        cloud
            .devices(&ada.token)
            .into_iter()
            .any(|held| held.id == device && held.cert.is_some())
    };
    assert!(certified(desktop.device), "the desktop's certificate");

    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);
    let linked = link_with(&ada, &typed(&new.code()), LAPTOP, YES);
    let shown = new.finish();
    assert!(!shown.success, "the new device succeeded: {}", shown.text);
    assert!(says(&shown.text, &[SAID]), "{}", shown.text);
    assert!(!linked.success, "skald link succeeded: {}", linked.text);
    assert!(
        cloud.count_sent(new.started, STORE_CERT) >= 1,
        "no certificate sent"
    );
    assert!(!certified(new.device), "a certificate over the limit");

    revoked_by_cli(&ada.machine, desktop.device);
    let later = run(&new.machine, &[crate::machine::DEVICES]);
    assert!(later.success, "{later:?}");
    assert!(
        certified(new.device),
        "the certificate is not stored: {later:?}"
    );
}
