//! The devices limit and certificates the master key issued: a registration at the limit is taken,
//! and a certificate that would give the account more devices with a certificate than the limit is
//! refused and not stored.

use crate::accounts::{held, put, register};
use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::issued_by_master;
use crate::upload::assert_over_limit;

/// At 2: Alice's laptop holds a certificate and her desktop is registered without one; her
/// desktop's certificate, issued by the master key, is stored. Her phone then registers at the
/// limit; its certificate, issued by the master key, is refused with `403` and the problem of the
/// limit, also when sent again, and the server holds none.
#[test]
fn at_the_limit_a_device_registers_and_its_master_key_certificate_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--devices-limit", "2"]);
    let alice = Person::new(&flow, 3_176_200_001, "alice");
    let desktop = &alice.uncertified;
    let stored = put(
        &flow,
        &alice.session,
        desktop,
        &issued_by_master(&alice, desktop, &alice.master),
    );
    assert_eq!(stored.status, 201, "the second device: {}", text(&stored));
    let phone = register(&alice, "Phone");

    let cert = issued_by_master(&alice, &phone, &alice.master);
    for what in ["over the limit", "again"] {
        let refused = put(&flow, &alice.session, &phone, &cert);
        assert_over_limit(&refused, what);
        assert_eq!(held(&alice, &phone), None, "{what}: stored");
    }
}

/// At 1: Alice's laptop holds the one certificate the limit allows, and her desktop registers at the
/// limit. Its certificate, issued by the master key, is refused as over the limit.
#[test]
fn at_a_limit_of_one_the_first_devices_account_registers_another_and_refuses_its_certificate() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--devices-limit", "1"]);
    let alice = Person::new(&flow, 3_176_200_101, "alice");
    let desktop = &alice.uncertified;

    let refused = put(
        &flow,
        &alice.session,
        desktop,
        &issued_by_master(&alice, desktop, &alice.master),
    );

    assert_over_limit(&refused, "over the limit");
    assert_eq!(held(&alice, desktop), None, "stored");
}
