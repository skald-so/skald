//! The server takes a device's KeyPackages and its last-resort one, and counts those that expire in
//! more than 7 days.

use crate::fixture::{Stand, expiring, fresh, hand_out_one, last_resort, mls, status, uploaded};

/// A device without KeyPackages has none unused and a last-resort one to replace. Once alice's
/// session uploads 3 of its laptop and a last-resort one, it has 3 and none to replace; 2 more that
/// expire within 7 days leave the count at 3, and 4 more that expire in 60 days make it 7.
#[test]
fn takes_a_devices_key_packages_and_counts_those_that_expire_in_more_than_seven_days() {
    let stand = Stand::new(3_130_010_001);
    let alice = &stand.team.alice;
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    assert_eq!(
        status(&alice.session, laptop),
        (0, true),
        "before an upload"
    );

    let last = last_resort(&mut device);
    uploaded(&alice.session, laptop, &fresh(&mut device, 3), Some(&last));
    assert_eq!(
        status(&alice.session, laptop),
        (3, false),
        "3 and a last-resort one"
    );

    uploaded(&alice.session, laptop, &expiring(&mut device, 2), None);
    assert_eq!(
        status(&alice.session, laptop),
        (3, false),
        "2 that expire within 7 days do not count"
    );

    uploaded(&alice.session, laptop, &fresh(&mut device, 4), None);
    assert_eq!(status(&alice.session, laptop), (7, false), "4 more");
}

/// A KeyPackage that expires within 7 days is still handed out while it holds: bob gets it.
#[test]
fn key_package_that_expires_within_seven_days_is_handed_out_while_it_holds() {
    let stand = Stand::new(3_130_010_101);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let mut device = mls(&alice.laptop);
    let soon = expiring(&mut device, 1);
    uploaded(&alice.session, alice.laptop.id, &soon, None);

    assert_eq!(
        hand_out_one(&bob.session, alice.laptop.id),
        Some(soon[0].clone())
    );
}

/// A new last-resort KeyPackage replaces the one the device had: once the old one was handed out,
/// the device has one to replace; after the upload it has none, and the new one is handed out each
/// time, never the old one.
#[test]
fn new_last_resort_key_package_replaces_the_old_one() {
    let stand = Stand::new(3_130_010_201);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    let old = last_resort(&mut device);
    uploaded(&alice.session, laptop, &[], Some(&old));
    assert_eq!(hand_out_one(&bob.session, laptop), Some(old.clone()));
    assert_eq!(
        status(&alice.session, laptop),
        (0, true),
        "the old one handed out"
    );

    let new = last_resort(&mut device);
    uploaded(&alice.session, laptop, &[], Some(&new));

    assert_eq!(
        status(&alice.session, laptop),
        (0, false),
        "the new one not handed out"
    );
    for _ in 0..2 {
        assert_eq!(hand_out_one(&bob.session, laptop), Some(new.clone()));
    }
}
