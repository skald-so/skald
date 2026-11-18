//! The devices limit counts certificates: with `--devices-limit`, a certificate that would give the
//! account more devices with a certificate than the limit is refused with `403` and the problem
//! `https://skald.so/problems/devices-limit`, and is not stored, also when two arrive at once. A
//! registration is no certificate, so one is taken at the limit. Devices without a certificate and
//! `service` devices do not count.

use std::sync::Barrier;
use std::thread;

use skald_proto::DeviceKind;

use crate::accounts::{by_laptop, certified, held, put, register};
use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::{Device, write_devices};
use crate::upload::{assert_over_limit, issued_by};

/// Checks that storing `device`'s certificate from `person`'s session is refused as over the limit
/// and stores nothing; `what` names the case.
#[track_caller]
fn assert_cert_over_limit(flow: &DeviceFlow, person: &Person, device: &Device, what: &str) {
    let response = put(flow, &person.session, device, &by_laptop(person, device));
    assert_over_limit(&response, what);
    assert_eq!(held(person, device), None, "{what}: stored");
}

/// At 2: alice's laptop holds a certificate, her desktop is registered without one. A third device
/// registers, also once her desktop's certificate is stored and she is at the limit; its
/// certificate is refused, also when sent again. Bob's account stores one of its own.
#[test]
fn at_the_limit_a_device_registers_and_its_certificate_is_refused() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--devices-limit", "2"]);
    let alice = Person::new(&flow, 3_175_300_001, "alice");
    let early = register(&alice, "Phone");

    let desktop = &alice.uncertified;
    let stored = put(&flow, &alice.session, desktop, &by_laptop(&alice, desktop));
    assert_eq!(stored.status, 201, "the second device: {}", text(&stored));
    assert_eq!(certified(&alice), 2);

    let late = register(&alice, "Tablet");
    assert_cert_over_limit(
        &flow,
        &alice,
        &early,
        "registered before the limit was reached",
    );
    assert_cert_over_limit(&flow, &alice, &late, "registered at the limit");
    assert_cert_over_limit(&flow, &alice, &late, "again");
    assert_eq!(certified(&alice), 2);

    let bob = Person::new(&flow, 3_175_300_002, "bob");
    let stored = put(
        &flow,
        &bob.session,
        &bob.uncertified,
        &by_laptop(&bob, &bob.uncertified),
    );
    assert_eq!(stored.status, 201, "another account: {}", text(&stored));
}

/// At 3, alice's account holds certificates of her laptop and of 4 `service` devices, and has 5
/// devices without one: her desktop's certificate is stored, the next one is refused.
#[test]
fn devices_without_a_certificate_and_service_devices_do_not_count() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--devices-limit", "3"]);
    let alice = Person::new(&flow, 3_175_300_011, "alice");
    let services: Vec<Device> = (0..4)
        .map(|at| Device::new(&format!("Agent {at}")))
        .collect();
    let certs: Vec<Vec<u8>> = services
        .iter()
        .map(|service| issued_by(service, alice.account, DeviceKind::Service, &alice.laptop))
        .collect();
    let rows: Vec<(&Device, Option<&[u8]>)> = services
        .iter()
        .zip(&certs)
        .map(|(service, cert)| (service, Some(cert.as_slice())))
        .collect();
    write_devices(&flow.stand.db, alice.account, DeviceKind::Service, &rows);
    let waiting: Vec<Device> = (0..5)
        .map(|at| register(&alice, &format!("Waiting {at}")))
        .collect();

    let desktop = &alice.uncertified;
    let stored = put(&flow, &alice.session, desktop, &by_laptop(&alice, desktop));
    assert_eq!(stored.status, 201, "the second: {}", text(&stored));
    let stored = put(
        &flow,
        &alice.session,
        &waiting[0],
        &by_laptop(&alice, &waiting[0]),
    );
    assert_eq!(stored.status, 201, "the third: {}", text(&stored));
    assert_cert_over_limit(&flow, &alice, &waiting[1], "the fourth");
}

/// At 2, in each of 3 accounts with one place left, the certificates of two devices arrive at once:
/// the server stores one and refuses the other as over the limit.
#[test]
fn of_two_certificates_at_once_for_the_last_place_one_is_stored() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--devices-limit", "2"]);
    let people: Vec<Person> = (0..3)
        .map(|at| Person::new(&flow, 3_175_300_021 + at, &format!("user{at}")))
        .collect();
    for person in &people {
        let phone = register(person, "Phone");
        let pair = [&person.uncertified, &phone];
        let certs = pair.map(|device| by_laptop(person, device));
        let barrier = Barrier::new(2);
        let statuses: Vec<(u16, String)> = thread::scope(|scope| {
            let sent: Vec<_> = pair
                .iter()
                .zip(&certs)
                .map(|(device, cert)| {
                    let barrier = &barrier;
                    let flow = &flow;
                    scope.spawn(move || {
                        barrier.wait();
                        let response = put(flow, &person.session, device, cert);
                        let over = response.status == 403;
                        if over {
                            assert_over_limit(&response, "the one refused");
                        }
                        (response.status, text(&response))
                    })
                })
                .collect();
            sent.into_iter()
                .map(|sent| sent.join().expect("a request"))
                .collect()
        });
        let mut codes: Vec<u16> = statuses.iter().map(|(status, _)| *status).collect();
        codes.sort_unstable();
        assert_eq!(codes, [201, 403], "{}: {statuses:?}", person.handle);
        assert_eq!(certified(person), 2, "{}", person.handle);
        let held_now = pair.map(|device| held(person, device).is_some());
        assert_eq!(
            held_now.iter().filter(|held| **held).count(),
            1,
            "{}: one of the two",
            person.handle
        );
    }
}
