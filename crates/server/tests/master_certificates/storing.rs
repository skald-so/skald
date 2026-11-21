//! A certificate the master key issued for a device of the account registered without one is
//! stored, and the same one again is taken as stored.

use crate::accounts::{held, put};
use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, text};
use crate::issued_by_master;

/// Alice's desktop, registered without a certificate, gets one the master key issued, as a device
/// linked with the recovery key issues its own: stored with `201`, and the account's devices then
/// give it as sent. Sent again, as after a lost answer, it is taken with `200`, and still held once.
#[test]
fn a_certificate_the_master_key_issued_is_stored_and_listed() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_176_100_001, "alice");
    let desktop = &alice.uncertified;
    let cert = issued_by_master(&alice, desktop, &alice.master);

    let stored = put(&flow, &alice.session, desktop, &cert);

    assert_eq!(stored.status, 201, "{}", text(&stored));
    assert_eq!(held(&alice, desktop), Some(cert.clone()), "the certificate");
    let again = put(&flow, &alice.session, desktop, &cert);
    assert_eq!(again.status, 200, "{}", text(&again));
    assert_eq!(held(&alice, desktop), Some(cert), "the certificate again");
}
