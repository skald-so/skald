//! Devices as plain HTTP shows them: which status a taken registration or master key answers, and
//! a `name` with U+0000. Every response is checked against the OpenAPI spec in `api/openapi.json`.

mod common;

use common::device_flow::{DeviceFlow, text};
use common::devices::{Device, Key, machine};

#[test]
fn first_registration_and_master_key_answer_201_and_a_repeat_200() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_666_001, "ada-status-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("Laptop");
    let registration = device.registration();
    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);

    for (status, what) in [(201, "first"), (200, "repeat")] {
        let registered = machine.register(&registration);
        assert_eq!(registered.status, status, "{what}: {}", text(&registered));
    }
    for (status, what) in [(201, "first"), (200, "repeat")] {
        let given = machine.give_master(&master, &cert);
        assert_eq!(given.status, status, "{what}: {}", text(&given));
    }
    let registered = machine.register(&registration);
    assert_eq!(registered.status, 200, "{}", text(&registered));
}

#[test]
fn a_name_with_u0000_is_registered_and_certified() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(3_120_666_002, "ada-nul-standin");
    let machine = machine(&flow, &ada);
    let device = Device::new("a\u{0}b");
    let master = Key::generate();
    let cert = device.cert(&machine.account, &master);

    let registered = machine.register(&device.registration());
    assert_eq!(registered.status, 201, "{}", text(&registered));
    let given = machine.give_master(&master, &cert);
    assert_eq!(given.status, 201, "{}", text(&given));
    let devices = machine.devices();
    assert_eq!(devices.get(&device.id.to_string()), Some(&Some(cert)));

    // The name is stored as sent: the same one without U+0000 is another name.
    let other = Device {
        name: "ab".to_owned(),
        ..device
    };
    let refused = machine.register(&other.registration());
    assert_eq!(refused.status, 409, "{}", text(&refused));
}
