//! The devices an account may have: with `--devices-limit`, `POST /devices` for an account that
//! has that many devices with a certificate is refused with `403` and the problem
//! `https://skald.so/problems/devices-limit`, and the device is not registered. Devices without a
//! certificate and `service` devices do not count, so registrations without a certificate cannot
//! fill the limit. Without the setting, an account registers any number of devices.

use serde_json::Value;
use skald_proto::DeviceKind;

use crate::api::{Person, assert_refused};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{DEVICES as REGISTER, Device, devices_of};
use crate::limits::{DEVICES, DEVICES_50, DEVICES_ENV, OVER_DEVICES, add_devices, restarted};

/// `POST /devices` of a new device of `person`'s account, on `flow`'s server; gives the device and
/// the answer.
fn register(flow: &DeviceFlow, person: &Person, name: &str) -> (Device, Response) {
    let device = Device::new(name);
    let session = person.session.at(flow);
    let response = session.call("post", REGISTER, Some(&device.registration()));
    (device, response)
}

/// Whether `person`'s account lists `device` in `GET /devices` on `flow`'s server.
fn lists(flow: &DeviceFlow, person: &Person, device: &Device) -> bool {
    let session = person.session.at(flow);
    devices_of(&session.call("get", REGISTER, None)).contains_key(&device.id.to_string())
}

/// Checks that `response` refuses a registration as over the devices limit: `403` with its
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
/// refuses it as over the limit and does not list it; `what` names the case.
#[track_caller]
fn assert_refused_registration(flow: &DeviceFlow, person: &Person, what: &str) {
    let (device, response) = register(flow, person, "One too many");
    assert_over_limit(&response, what);
    assert!(
        !lists(flow, person, &device),
        "{what}: the device is registered"
    );
}

/// Registers a new device of `person`'s account on `flow`'s server and checks that the server
/// takes it and lists it; `what` names the case.
#[track_caller]
fn assert_registers(flow: &DeviceFlow, person: &Person, what: &str) {
    let (device, response) = register(flow, person, "Another");
    assert!(
        matches!(response.status, 200 | 201),
        "{what}: taken, not {}",
        text(&response)
    );
    assert!(
        lists(flow, person, &device),
        "{what}: the device is not listed"
    );
}

/// With the setting at 50, an account with 49 certified `client` devices, besides devices without a
/// certificate and certified `service` devices, registers new devices, which have no certificate.
/// With a 50th certified one, its next registration is refused, also when asked again, while
/// another account still registers.
#[test]
fn with_50_certified_devices_the_next_registration_is_refused() {
    let limit = DEVICES_50.to_string();
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES, &limit]);
    let db = &flow.stand.db;
    // Its laptop is certified, its desktop is not.
    let alice = Person::new(&flow, 3_121_310_201, "alice");
    add_devices(db, &alice, DEVICES_50 - 2, DeviceKind::Client, true);
    add_devices(db, &alice, 10, DeviceKind::Client, false);
    add_devices(db, &alice, 3, DeviceKind::Service, true);

    for at in 0..5 {
        assert_registers(&flow, &alice, &format!("registration {at} at 49"));
    }
    add_devices(db, &alice, 1, DeviceKind::Client, true);

    assert_refused_registration(&flow, &alice, "at 50");
    assert_refused_registration(&flow, &alice, "at 50, again");
    let bob = Person::new(&flow, 3_121_310_202, "bob");
    assert_registers(&flow, &bob, "another account");
}

/// The setting gives the number: at 2, an account with its certified laptop registers another
/// device, and once a second device of it is certified, its next registration is refused.
#[test]
fn the_setting_gives_the_number_of_devices() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES, "2"]);
    let alice = Person::new(&flow, 3_121_310_211, "alice");

    assert_registers(&flow, &alice, "at 1");
    add_devices(&flow.stand.db, &alice, 1, DeviceKind::Client, true);
    assert_refused_registration(&flow, &alice, "at 2");
}

/// Without the setting, an account with 50 certified devices registers another, and one with 60
/// registers too. Started again on the same database with the setting at 50, by its environment
/// variable, the server refuses the account's next registration, while a new account registers.
#[test]
fn without_the_setting_a_device_registers_next_to_50_certified_ones() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_121_310_221, "alice");
    add_devices(
        &flow.stand.db,
        &alice,
        DEVICES_50 - 1,
        DeviceKind::Client,
        true,
    );
    assert_registers(&flow, &alice, "next to 50");
    add_devices(&flow.stand.db, &alice, 10, DeviceKind::Client, true);
    assert_registers(&flow, &alice, "next to 60");

    let limit = DEVICES_50.to_string();
    let flow = restarted(flow.stand, &[(DEVICES_ENV, &limit)]);
    assert_refused_registration(&flow, &alice, "after the restart");
    let bob = Person::new(&flow, 3_121_310_222, "bob");
    assert_registers(&flow, &bob, "a new account after the restart");
}
