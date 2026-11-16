//! Bounds on what a device holds: the same KeyPackage uploaded again is one KeyPackage, a device
//! holds at most 100 unused ones, the last-resort one aside, and a last-resort one carries the
//! `last_resort` extension; expired ones are deleted.

use crate::api::assert_refused;
use crate::fixture::{Stand, fresh, hand_out_one, last_resort, mls, status, upload, uploaded};

/// The most unused KeyPackages a device holds.
const HELD: usize = 100;

/// Two KeyPackages uploaded twice count as two, and each is handed out once.
#[test]
fn same_key_package_uploaded_again_is_one_key_package() {
    let stand = Stand::new(3_130_060_001);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = alice.laptop.id;
    let two = fresh(&mut mls(&alice.laptop), 2);
    uploaded(&alice.session, laptop, &two, None);
    uploaded(&alice.session, laptop, &two, None);

    assert_eq!(status(&alice.session, laptop), (2, true));
    let first = hand_out_one(&bob.session, laptop).expect("one");
    let second = hand_out_one(&bob.session, laptop).expect("the other");
    assert_ne!(first, second, "a KeyPackage handed out twice");
    assert_eq!(hand_out_one(&bob.session, laptop), None);
}

/// A device with 100 unused KeyPackages takes no more: the upload of one more is refused, whole.
#[test]
fn device_holds_at_most_a_hundred_unused_key_packages() {
    let stand = Stand::new(3_130_060_101);
    let alice = &stand.team.alice;
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    uploaded(&alice.session, laptop, &fresh(&mut device, HELD - 1), None);

    assert_refused(
        &upload(&alice.session, laptop, &fresh(&mut device, 2), None),
        "101 KeyPackages",
    );
    assert_eq!(
        status(&alice.session, laptop).0,
        99,
        "a refused upload stores none"
    );
    uploaded(&alice.session, laptop, &fresh(&mut device, 1), None);
    assert_eq!(status(&alice.session, laptop).0, 100);
}

/// An expired KeyPackage is deleted once the device's KeyPackages are counted, and once one is
/// asked for: the database no longer holds it.
#[test]
fn expired_key_packages_are_deleted() {
    let stand = Stand::new(3_130_060_201);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);

    let counted = fresh(&mut device, 1);
    uploaded(&alice.session, laptop, &counted, None);
    stand.age("61 days");
    assert!(stand.holds(&counted[0]), "before the count");
    assert_eq!(status(&alice.session, laptop), (0, true));
    assert!(!stand.holds(&counted[0]), "deleted by the count");

    let asked = fresh(&mut device, 1);
    uploaded(&alice.session, laptop, &asked, None);
    stand.age("61 days");
    assert_eq!(hand_out_one(&bob.session, laptop), None);
    assert!(!stand.holds(&asked[0]), "deleted by the hand-out");
}

/// A last-resort KeyPackage without the `last_resort` extension is refused; as another one, it is
/// taken.
#[test]
fn last_resort_key_package_without_the_extension_is_refused() {
    let stand = Stand::new(3_130_060_301);
    let alice = &stand.team.alice;
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    let plain = fresh(&mut device, 1).remove(0);

    assert_refused(
        &upload(&alice.session, laptop, &[], Some(&plain)),
        "a last-resort one without the extension",
    );
    assert_eq!(status(&alice.session, laptop), (0, true));
    uploaded(
        &alice.session,
        laptop,
        &[plain],
        Some(&last_resort(&mut device)),
    );
    assert_eq!(status(&alice.session, laptop), (1, false));
}
