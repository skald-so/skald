//! A revoked device's connections: they close on every instance, and a new one is refused until a
//! master-key revocation voids the revocation. Checks 1 and 2 are not made again.

use std::time::Instant;

use crate::common::devices::Key;
use crate::fixture::Fixture;
use crate::kit::{
    assert_closes, assert_device_refused, assert_open, by_bobs_laptop, connect, sorted,
};

/// bob's phone, certified by bob's laptop, is connected to two instances when the laptop revokes
/// it on one of them. Both connections close within a second, a new one to either instance is
/// refused, and the laptop still connects.
#[test]
fn a_revoked_devices_connections_close_on_every_instance_and_its_proof_is_refused() {
    let f = Fixture::new(3_131_320_401);
    let t = &f.team;
    let flow = &t.flow;
    let bob = &t.bob;
    let phone = by_bobs_laptop(&f, "Phone");
    let here = &flow.stand.server;
    let other = flow.stand.instance(&[], &[]);
    let mut on_here = connect(here, flow, bob, &phone);
    assert_open(&mut on_here);
    let mut on_other = connect(&other, flow, bob, &phone);
    assert_open(&mut on_other);

    bob.revoke(&bob.by_device(&bob.laptop, phone.id));
    let since = Instant::now();
    assert_closes(&mut on_other, since, "on the other instance");
    assert_closes(&mut on_here, since, "on the instance that stored it");
    for server in [here, &other] {
        assert_device_refused(&mut connect(server, flow, bob, &phone));
    }
    assert_open(&mut connect(&other, flow, bob, &bob.laptop));
}

/// bob's laptop revokes bob's phone, which it certified, and the phone is refused. Once bob's
/// master key revokes the laptop, keeping the phone, it voids the laptop's revocation: the phone
/// connects again and the laptop is refused. Both revocations stay given.
#[test]
fn a_master_key_revocation_that_voids_a_revocation_lets_its_device_connect_again() {
    let f = Fixture::new(3_131_320_411);
    let t = &f.team;
    let flow = &t.flow;
    let bob = &t.bob;
    let here = &flow.stand.server;
    let phone = by_bobs_laptop(&f, "Phone");
    let by_laptop = bob.by_device(&bob.laptop, phone.id);
    bob.revoke(&by_laptop);
    assert_device_refused(&mut connect(here, flow, bob, &phone));

    let by_master = bob.by_master(bob.laptop.id, &[phone.id]);
    bob.revoke(&by_master);
    assert_open(&mut connect(here, flow, bob, &phone));
    assert_device_refused(&mut connect(here, flow, bob, &bob.laptop));
    assert_eq!(
        bob.session.revocations(),
        sorted(&[&by_laptop, &by_master]),
        "bob's devices"
    );
}

/// A device of bob's account whose certificate another master key issued, which the server holds
/// without having checked it, still connects once a revocation of another device of the account
/// is stored: the server checks only revocations against it.
#[test]
fn a_certificate_chained_to_another_master_key_still_gets_in_beside_a_revocation() {
    let f = Fixture::new(3_131_320_421);
    let t = &f.team;
    let flow = &t.flow;
    let bob = &t.bob;
    let here = &flow.stand.server;
    let stray = bob.certified_by_key(f.db(), &Key::generate(), "Stray");
    let phone = by_bobs_laptop(&f, "Phone");
    bob.revoke(&bob.by_device(&bob.laptop, phone.id));
    assert_device_refused(&mut connect(here, flow, bob, &phone));
    assert_open(&mut connect(here, flow, bob, &stray));
    assert_open(&mut connect(here, flow, bob, &bob.laptop));
}
