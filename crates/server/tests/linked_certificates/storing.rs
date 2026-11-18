//! A session of the account stores the certificate a linked `client` device issued its new device:
//! the account's sessions then get it as sent, the same certificate again is a retry, a chain
//! through a linked device is valid too, and the new device's link code run ends.

use crate::accounts::{by_laptop, held, put, register};
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::{DEVICES, devices_of};
use crate::kit::{Link, assert_refused_link, fresh_session, grant, hello, reply};
use crate::upload::issued_by;
use skald_proto::DeviceKind;

#[test]
fn a_certificate_a_linked_device_issued_is_stored_and_given_as_sent() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let alice = crate::api::Person::new(&flow, 3_175_100_001, "alice");
    let bob = crate::api::Person::new(&flow, 3_175_100_002, "bob");
    let desktop = &alice.uncertified;
    let cert = by_laptop(&alice, desktop);

    let stored = put(&flow, &alice.session, desktop, &cert);
    assert_eq!(stored.status, 201, "stored: {}", text(&stored));
    assert_eq!(held(&alice, desktop), Some(cert.clone()), "as sent");
    assert_eq!(held(&alice, &alice.laptop), Some(alice.cert.clone()));

    // A retry after a lost answer.
    let again = put(&flow, &alice.session, desktop, &cert);
    assert_eq!(
        again.status,
        200,
        "the same certificate again: {}",
        text(&again)
    );
    assert_eq!(held(&alice, desktop), Some(cert), "unchanged");

    let bobs = devices_of(&bob.session.call("get", DEVICES, None));
    assert!(
        !bobs.contains_key(&desktop.id.to_string()),
        "bob's account lists alice's desktop: {bobs:?}"
    );
    assert_eq!(held(&bob, &bob.uncertified), None, "bob's desktop");
}

/// The chain master key, laptop, desktop, phone: once the desktop holds its certificate, it issues
/// the phone's, which the server takes.
#[test]
fn a_certificate_issued_by_a_device_the_laptop_linked_is_stored() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let alice = crate::api::Person::new(&flow, 3_175_100_011, "alice");
    let desktop = &alice.uncertified;
    let phone = register(&alice, "Phone");
    let by_desktop = issued_by(&phone, alice.account, DeviceKind::Client, desktop);

    // Until the desktop holds a certificate, its issuer has none.
    let early = put(&flow, &alice.session, &phone, &by_desktop);
    assert!(
        (400..500).contains(&early.status) && early.status != 401,
        "issued by a device without a certificate: {}",
        text(&early)
    );
    assert_eq!(held(&alice, &phone), None);

    let stored = put(&flow, &alice.session, desktop, &by_laptop(&alice, desktop));
    assert_eq!(stored.status, 201, "the desktop: {}", text(&stored));
    let stored = put(&flow, &alice.session, &phone, &by_desktop);
    assert_eq!(stored.status, 201, "the phone: {}", text(&stored));
    assert_eq!(held(&alice, &phone), Some(by_desktop));
}

/// The new device's machine stores its own certificate from its session, which ends its run:
/// its device no longer waits, the run's later messages are refused, and the certified device opens
/// no other run.
#[test]
fn the_new_devices_session_stores_its_certificate_which_ends_its_run() {
    let l = Link::start(3_175_100_021);
    let (session, _) = l.open();
    assert!(
        matches!(
            l.alice.session.send_message(&reply(&session)).status,
            200 | 201
        ),
        "the reply"
    );
    let desktop = &l.alice.uncertified;
    let cert = by_laptop(&l.alice, desktop);

    let stored = put(&l.flow, &l.new_side, desktop, &cert);
    assert_eq!(stored.status, 201, "stored: {}", text(&stored));
    l.alice.session.until_not_waiting(l.desktop());
    assert_refused_link(
        &l.alice.session.send_message(&grant(&session)),
        "a grant after the certificate",
    );
    assert_refused_link(
        &l.new_side.send_hello(l.desktop(), &hello(&fresh_session())),
        "a hello naming the certified device",
    );
    assert_eq!(held(&l.alice, desktop), Some(cert.clone()));

    // The issuing device's session, retrying for it.
    let again = put(&l.flow, &l.alice.session, desktop, &cert);
    assert_eq!(
        again.status,
        200,
        "a retry from another session: {}",
        text(&again)
    );
}
