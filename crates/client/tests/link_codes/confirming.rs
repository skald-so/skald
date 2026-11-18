//! `skald link` tells its person to enter only a code shown on their own device before it takes
//! one, shows the new device's name and kind for its person to confirm and issues nothing when they
//! decline; an entry that is not a code starts no run.

use std::time::Instant;

use crate::cloud::Cloud;
use crate::linking::{
    NO_CONFIRM, NewDevice, OWN_DEVICE, RELAYED, STORE_CERT, YES, link, link_failing, link_with,
    start_link, typed,
};
use crate::world::World;

/// The host name of Ada's new machine.
const LAPTOP: &str = "ada-laptop";

/// The requests to the link code relay the relay saw since `since`.
fn relayed_since(cloud: &Cloud, since: Instant) -> Vec<String> {
    let path = RELAYED.to_lowercase();
    cloud
        .relay
        .sent_since(since)
        .into_iter()
        .map(|sent| sent.head)
        .filter(|head| head.contains(&path))
        .collect()
}

/// `skald link` says to enter only a code of the person's own device before it sends anything.
/// Shown the new device's name and kind, its person declines: the run ends, the new device says it
/// failed and has no certificate, and its next run, with the same code, links it.
#[test]
fn the_linked_device_reminds_before_the_code_and_issues_nothing_when_declined() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_301_001, "ada-declines-once", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);

    let before = cloud.relay.mark();
    let mut asking = start_link(&ada, &[]);
    assert!(
        relayed_since(cloud, before).is_empty(),
        "sent before the person typed a code: {:?}",
        relayed_since(cloud, before)
    );
    asking.type_line(&typed(&new.code()));
    asking.wait_for("the new device's name and kind", |shown| {
        shown.contains(LAPTOP) && shown.to_lowercase().contains("client")
    });
    asking.type_line(NO_CONFIRM);
    let declined = asking.finish();
    assert!(!declined.success, "skald link succeeded: {}", declined.text);
    assert!(
        declined.text.to_lowercase().contains(OWN_DEVICE),
        "{}",
        declined.text
    );

    new.wait_for_failures(1);
    let certified = cloud
        .devices(&ada.token)
        .into_iter()
        .any(|device| device.id == new.device && device.cert.is_some());
    assert!(!certified, "a declined new device has a certificate");
    assert_eq!(cloud.count_sent(new.started, STORE_CERT), 0, "it sent one");

    link(cloud, &ada, &mut new);
}

/// Entries that are not 6 characters of the code's alphabet, read as entry reads them, each make
/// `skald link` exit failing without a request to the link code relay; they spend neither the
/// new device's attempts nor the linked device's, so the code then links at once, also typed with
/// `-` and in lowercase.
#[test]
fn a_malformed_entry_starts_no_run() {
    let world = World::start();
    let cloud = &world.cloud;
    let ada = world.sign_up(4_175_301_011, "ada-mistypes", "ada-workstation");
    let mut new = NewDevice::start(&world, &ada, LAPTOP);
    new.open_run(cloud, &ada);

    for entry in ["ABC-DE", "ABCD-EFG", "AB9-DEF", "ABC_DEF", "ABC.DE2"] {
        let before = cloud.relay.mark();
        link_failing(&ada, entry);
        assert_eq!(
            relayed_since(cloud, before),
            Vec::<String>::new(),
            "{entry:?} started a run"
        );
    }
    assert!(
        !crate::linking::says(&new.shown(), &["fail"]),
        "{}",
        new.shown()
    );

    let code = new.code().to_lowercase();
    let linked = link_with(&ada, &code, LAPTOP, YES);
    assert!(linked.success, "{}", linked.text);
    assert!(new.finish().success, "the new device failed");
}
