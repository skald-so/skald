//! Where else the server checks a device's certificate: a revoked device has none, so it does not
//! count towards the devices limit and signs no roster revision the server stores.

use serde_json::Value;
use skald_proto::{DeviceKind, Role};

use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json};
use crate::common::devices::{DEVICES as REGISTER, Device};
use crate::fixture::Fixture;
use crate::limits::{DEVICES, OVER_DEVICES, add_devices};
use crate::rosters::{member, next};

/// The status and problem type of `POST /devices` of a new device of `person`'s account.
fn register(person: &Person, name: &str) -> (u16, Value) {
    let device = Device::new(name);
    let response = person
        .session
        .call("post", REGISTER, Some(&device.registration()));
    let problem = json(&response).get("type").cloned().unwrap_or(Value::Null);
    (response.status, problem)
}

/// With the devices limit at 2, alice's account has its laptop and a tablet certified, so its next
/// registration is refused. Once alice's master key revokes the tablet, the account registers a
/// device in its place.
#[test]
fn a_revoked_device_frees_its_place_under_the_devices_limit() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", DEVICES, "2"]);
    let alice = Person::new(&flow, 3_131_320_501, "alice");
    let tablet = add_devices(&flow.stand.db, &alice, 1, DeviceKind::Client, true).remove(0);
    let over = (403, Value::from(OVER_DEVICES));
    assert_eq!(register(&alice, "One too many"), over, "at the limit");

    alice.revoke(&alice.by_master(tablet.id, &[]));
    assert_eq!(
        register(&alice, "In its place"),
        (201, Value::Null),
        "with the tablet revoked"
    );
}

/// Once alice's master key revokes alice's laptop, the server refuses the next revision of `acme`
/// signed by the laptop, and stores it signed by the laptop of bob, the other admin.
#[test]
fn a_roster_revision_signed_by_a_revoked_device_is_refused() {
    let f = Fixture::new(3_131_320_511);
    let t = &f.team;
    let last = f.revisions.last().expect("a revision");
    let mut by_alice = next(last, &t.alice.laptop);
    by_alice.members.push(member(&f.eve, Role::Member));
    let mut by_bob = by_alice.clone();
    by_bob.signer = t.bob.laptop.id;

    t.alice.revoke(&t.alice.by_master(t.alice.laptop.id, &[]));
    let revision = t.alice.sign(&by_alice);
    t.refused(
        &t.alice,
        &revision,
        &f.revisions,
        "signed by a revoked laptop",
    );
    t.stored_by(&t.bob, &t.bob.sign(&by_bob));
}
