//! A session registers a `service` device of its account and stores its certificate, issued by a
//! `client` device of the account; the session's device stays the one it registered before.

use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::{Owner, device_session_names, service_registration};

/// Alice's session registers a `service` device, and the same registration again is taken as
/// stored; the account lists it without a certificate. Its certificate, issued by her laptop, is
/// stored, and sent again is taken as stored; the account lists it as sent.
#[test]
fn session_registers_a_service_device_and_stores_its_certificate_from_a_client_device() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let alice = Owner::new(&flow, 3_135_301_001, "alice-service-standin");

    let bot = alice.register_service("Deploy bot");
    let again = alice.register(&service_registration(&bot));
    assert_eq!(
        again.status,
        200,
        "the same registration again: {}",
        text(&again)
    );
    assert_eq!(
        alice.cert_of(bot.id),
        None,
        "a certificate before the upload"
    );

    let cert = alice.service_cert(&bot, &alice.laptop);
    let stored = alice.upload(bot.id, &cert);
    assert_eq!(stored.status, 201, "the certificate: {}", text(&stored));
    let again = alice.upload(bot.id, &cert);
    assert_eq!(
        again.status,
        200,
        "the same certificate again: {}",
        text(&again)
    );
    assert_eq!(alice.cert_of(bot.id), Some(cert), "not the bytes as sent");
}

/// Alice's CLI session registered her laptop; registering a `service` device and storing its
/// certificate from that session leave the session's device as it was: Access lists the session
/// under the laptop, never under the service device.
#[test]
fn registering_a_service_device_leaves_the_session_s_device() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let alice = Owner::new(&flow, 3_135_301_101, "alice-access-standin");
    let cookie = &alice.machine.cookie;
    let laptop = Some(alice.laptop.name.clone());
    assert_eq!(
        device_session_names(&flow, cookie),
        std::slice::from_ref(&laptop)
    );

    let bot = alice.register_service("Deploy bot");
    assert_eq!(
        device_session_names(&flow, cookie),
        std::slice::from_ref(&laptop),
        "after the service device's registration"
    );
    let stored = alice.upload(bot.id, &alice.service_cert(&bot, &alice.laptop));
    assert_eq!(stored.status, 201, "the certificate: {}", text(&stored));
    assert_eq!(
        device_session_names(&flow, cookie),
        [laptop],
        "after the service device's certificate"
    );
}
