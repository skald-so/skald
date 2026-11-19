//! The devices an account may have: with `--devices-limit`, a certificate that would give the
//! account more devices with a certificate than that, `PUT /devices/{device}/cert`, is refused with
//! `403` and the problem `https://skald.so/problems/devices-limit`, and is not stored. A
//! registration, `POST /devices`, has no certificate, so it is taken at the limit. Devices without a
//! certificate and `service` devices do not count. Without the setting, an account holds any
//! number of certificates.

use serde_json::Value;
use skald_proto::DeviceKind;

use crate::api::{Person, assert_refused};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{DEVICES as REGISTER, Device, devices_of};
use crate::limits::{DEVICES, DEVICES_50, DEVICES_ENV, OVER_DEVICES, add_devices, restarted};
use crate::upload::{issued_by, put_cert};

/// `POST /devices` of a new device of `person`'s account, on `flow`'s server, which the server
/// must take and list without a certificate; gives the device.
#[track_caller]
fn register(flow: &DeviceFlow, person: &Person, name: &str) -> Device {
    let device = Device::new(name);
    let session = person.session.at(flow);
    let response = session.call("post", REGISTER, Some(&device.registration()));
    assert!(
        matches!(response.status, 200 | 201),
        "{name}: registered, not {}",
        text(&response)
    );
    assert_eq!(
        held(flow, person, &device),
        None,
        "{name}: listed without a certificate"
    );
    device
}

/// The certificate `person`'s account holds for `device` on `flow`'s server, which must list it.
#[track_caller]
fn held(flow: &DeviceFlow, person: &Person, device: &Device) -> Option<Vec<u8>> {
    let session = person.session.at(flow);
    let listed = devices_of(&session.call("get", REGISTER, None));
    listed
        .get(&device.id.to_string())
        .unwrap_or_else(|| panic!("{} is not listed", device.name))
        .clone()
}

/// `PUT /devices/<device>/cert` on `flow`'s server of `device`'s certificate on `person`'s
/// account, issued by `person`'s laptop.
fn certify(flow: &DeviceFlow, person: &Person, device: &Device) -> Response {
    let cert = issued_by(device, person.account, DeviceKind::Client, &person.laptop);
    let session = person.session.at(flow);
    put_cert(flow.stand.server.base(), session.token(), device.id, &cert)
}

/// Checks that `response` refuses a certificate as over the devices limit: `403` with its
/// problem; `what` names the case.
#[track_caller]
fn assert_over_limit(response: &Response, what: &str) {
    assert_refused(response, what);
    let problem = (response.status, json(response)["type"].clone());
    assert_eq!(
        problem,
        (403, Value::from(OVER_DEVICES)),
        "{what}: {}",
        text(response)
    );
}

/// Registers a new device of `person`'s account on `flow`'s server and checks that the server
/// refuses its certificate as over the limit, also when sent again, and does not store it; `what`
/// names the case.
#[track_caller]
fn assert_refused_certificate(flow: &DeviceFlow, person: &Person, what: &str) {
    let device = register(flow, person, "One too many");
    assert_over_limit(&certify(flow, person, &device), what);
    assert_over_limit(&certify(flow, person, &device), &format!("{what}, again"));
    assert_eq!(held(flow, person, &device), None, "{what}: stored");
}

/// Registers a new device of `person`'s account on `flow`'s server and checks that the server
/// stores its certificate; `what` names the case.
#[track_caller]
fn assert_certifies(flow: &DeviceFlow, person: &Person, what: &str) {
    let device = register(flow, person, "Another");
    let response = certify(flow, person, &device);
    assert_eq!(
        response.status,
        201,
        "{what}: stored, not {}",
        text(&response)
    );
    assert!(held(flow, person, &device).is_some(), "{what}: not held");
}

/// With the setting at 50, an account with 48 certified `client` devices and its certified laptop,
/// besides devices without a certificate and certified `service` devices, registers new devices
/// and stores a 50th certificate. At 50 it still registers new devices, while their certificates
/// are refused, also when sent again, and another account stores one.
#[test]
fn with_50_certified_devices_the_next_certificate_is_refused() {
    let limit = DEVICES_50.to_string();
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES, &limit]);
    let db = &flow.stand.db;
    // Its laptop is certified, its desktop is not.
    let alice = Person::new(&flow, 3_121_310_201, "alice");
    add_devices(db, &alice, DEVICES_50 - 2, DeviceKind::Client, true);
    add_devices(db, &alice, 10, DeviceKind::Client, false);
    add_devices(db, &alice, 3, DeviceKind::Service, true);

    for at in 0..5 {
        register(&flow, &alice, &format!("Registration {at} at 49"));
    }
    assert_certifies(&flow, &alice, "the 50th");
    for at in 0..3 {
        register(&flow, &alice, &format!("Registration {at} at 50"));
    }

    assert_refused_certificate(&flow, &alice, "at 50");
    let bob = Person::new(&flow, 3_121_310_202, "bob");
    assert_certifies(&flow, &bob, "another account");
}

/// The setting gives the number: at 2, an account with its certified laptop stores the certificate
/// of another device, and refuses the next one's.
#[test]
fn the_setting_gives_the_number_of_devices() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES, "2"]);
    let alice = Person::new(&flow, 3_121_310_211, "alice");

    assert_certifies(&flow, &alice, "at 1");
    assert_refused_certificate(&flow, &alice, "at 2");
}

/// Without the setting, an account with 50 certified devices stores another certificate, and one
/// with 61 too. Started again on the same database with the setting at 50, by its environment
/// variable, the server refuses the account's next certificate, while a new account stores one.
#[test]
fn without_the_setting_a_device_is_certified_next_to_50_certified_ones() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_121_310_221, "alice");
    add_devices(
        &flow.stand.db,
        &alice,
        DEVICES_50 - 1,
        DeviceKind::Client,
        true,
    );
    assert_certifies(&flow, &alice, "next to 50");
    add_devices(&flow.stand.db, &alice, 10, DeviceKind::Client, true);
    assert_certifies(&flow, &alice, "next to 61");

    let limit = DEVICES_50.to_string();
    let flow = restarted(flow.stand, &[(DEVICES_ENV, &limit)]);
    assert_refused_certificate(&flow, &alice, "after the restart");
    let bob = Person::new(&flow, 3_121_310_222, "bob");
    assert_certifies(&flow, &bob, "a new account after the restart");
}
