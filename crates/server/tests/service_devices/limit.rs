//! `service` devices stay outside `--devices-limit`, at registration and at the certificate. The
//! limit applies where the server takes a certificate: registering certifies nothing.

use serde_json::Value;
use skald_proto::DeviceKind;

use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{Device, cert_of};
use crate::{DEVICES_LIMIT, OVER_DEVICES, Owner, body_by};

/// With the limit at 1, Alice's account is at it with her certified laptop. Two `service` devices
/// register, and their certificates, issued by the laptop, are stored.
#[test]
fn at_the_devices_limit_service_devices_register_and_take_their_certificates() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES_LIMIT, "1"]);
    let alice = Owner::new(&flow, 3_135_303_001, "alice-limit-standin");

    for name in ["Deploy bot", "Release bot"] {
        let bot = alice.register_service(name);
        let cert = alice.service_cert(&bot, &alice.laptop);
        let stored = alice.upload(bot.id, &cert);
        assert_eq!(
            stored.status,
            201,
            "{name}'s certificate: {}",
            text(&stored)
        );
        assert_eq!(alice.cert_of(bot.id), Some(cert), "{name}'s certificate");
    }
}

/// With the limit at 1 and Alice's account at it, besides a certified `service` device: a `client`
/// device registers, as registering certifies nothing, and its certificate, issued by the laptop,
/// is refused with `devices-limit` and not stored.
#[test]
fn at_the_devices_limit_a_client_device_registers_and_its_certificate_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES_LIMIT, "1"]);
    let alice = Owner::new(&flow, 3_135_303_101, "alice-over-standin");
    let bot = alice.register_service("Deploy bot");
    let stored = alice.upload(bot.id, &alice.service_cert(&bot, &alice.laptop));
    assert_eq!(
        stored.status,
        201,
        "the bot's certificate: {}",
        text(&stored)
    );

    let desktop = Device::new("Desktop");
    let registered = alice.register(&desktop.registration());
    assert_eq!(
        registered.status,
        201,
        "a registration at the limit: {}",
        text(&registered)
    );
    let body = body_by(
        &desktop,
        alice.account(),
        DeviceKind::Client,
        alice.laptop.id,
    );
    let cert = cert_of(&body, &alice.laptop.signing.key, &desktop.signing.key);
    let refused = alice.upload(desktop.id, &cert);

    let problem = (refused.status, json(&refused)["type"].clone());
    assert_eq!(
        problem,
        (403, Value::from(OVER_DEVICES)),
        "{}",
        text(&refused)
    );
    assert_eq!(alice.cert_of(desktop.id), None, "the certificate is stored");
}
