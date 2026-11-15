//! Handing out KeyPackages: each other one once, then deleted; the last-resort one only when no
//! other remains, each time; no expired one; one for each device asked for; only to sessions of
//! accounts that share a space with the device.

use std::collections::BTreeSet;

use crate::fixture::{Stand, fresh, hand_out, hand_out_one, last_resort, mls, status, uploaded};

/// Alice's laptop has 3 KeyPackages and a last-resort one. Bob gets each of the 3 once, in three
/// requests, and the database no longer holds them; then the last-resort one each time, which the
/// database still holds. Alice's count goes down to 0, and the last-resort one, handed out, is to
/// be replaced.
#[test]
fn each_other_key_package_is_handed_out_once_then_the_last_resort_one_each_time() {
    let stand = Stand::new(3_130_030_001);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    let others = fresh(&mut device, 3);
    let last = last_resort(&mut device);
    uploaded(&alice.session, laptop, &others, Some(&last));

    let mut handed = Vec::new();
    for left in [2, 1, 0] {
        let one = hand_out_one(&bob.session, laptop).expect("a KeyPackage");
        assert!(others.contains(&one), "not one of the others uploaded");
        assert!(!handed.contains(&one), "a KeyPackage handed out twice");
        handed.push(one);
        assert_eq!(status(&alice.session, laptop), (left, false), "{left} left");
    }
    for one in &handed {
        assert!(
            !stand.holds(one),
            "the database holds a handed-out KeyPackage"
        );
    }

    for _ in 0..3 {
        assert_eq!(
            hand_out_one(&bob.session, laptop),
            Some(last.clone()),
            "the last-resort one once no other remains"
        );
    }
    assert!(stand.holds(&last), "the last-resort one was deleted");
    assert_eq!(
        status(&alice.session, laptop),
        (0, true),
        "the last-resort one handed out"
    );
}

/// Bob asks at once for alice's laptop, carol's laptop and dave's, and alice's device without a
/// certificate: he gets one KeyPackage of each laptop in acme, each of its own device, and none of
/// dave's, who shares no space with him, or of the device without any.
#[test]
fn one_key_package_is_handed_out_for_each_device_asked_for() {
    let stand = Stand::new(3_130_030_101);
    let team = &stand.team;
    let mut uploads = Vec::new();
    for person in [&team.alice, &team.carol, &team.dave] {
        let mut device = mls(&person.laptop);
        let others = fresh(&mut device, 2);
        uploaded(&person.session, person.laptop.id, &others, None);
        uploads.push((person.laptop.id, others));
    }

    let handed = hand_out(
        &team.bob.session,
        &[
            team.alice.laptop.id,
            team.carol.laptop.id,
            team.dave.laptop.id,
            team.alice.uncertified.id,
        ],
    );

    let got: BTreeSet<_> = handed.keys().copied().collect();
    assert_eq!(
        got,
        BTreeSet::from([team.alice.laptop.id, team.carol.laptop.id]),
        "the devices with a KeyPackage for bob"
    );
    for (device, others) in &uploads[..2] {
        assert!(
            others.contains(&handed[device]),
            "another device's KeyPackage"
        );
    }
    assert_eq!(status(&team.alice.session, team.alice.laptop.id).0, 1);
    assert_eq!(status(&team.carol.session, team.carol.laptop.id).0, 1);
    assert_eq!(status(&team.dave.session, team.dave.laptop.id).0, 2);
}

/// Dave, in no space, gets none of alice's laptop's KeyPackages, and alice's count stays; once a
/// revision adds him to acme, he gets one.
#[test]
fn no_key_package_goes_to_an_account_that_shares_no_space_with_the_device() {
    let mut stand = Stand::new(3_130_030_201);
    let laptop = stand.team.alice.laptop.id;
    let mut device = mls(&stand.team.alice.laptop);
    let others = fresh(&mut device, 2);
    let last = last_resort(&mut device);
    uploaded(&stand.team.alice.session, laptop, &others, Some(&last));

    assert_eq!(
        hand_out_one(&stand.team.dave.session, laptop),
        None,
        "outside acme"
    );
    assert_eq!(status(&stand.team.alice.session, laptop), (2, false));

    stand.add_dave();
    let one = hand_out_one(&stand.team.dave.session, laptop).expect("a KeyPackage in acme");
    assert!(others.contains(&one), "not one of the others uploaded");
}

/// A KeyPackage whose time has passed is not handed out: alice's laptop's only one, its times moved
/// back 61 days, is not; nor, after a fresh one and the last-resort one are handed out, is the
/// last-resort one once its times are moved back too.
#[test]
fn expired_key_packages_are_not_handed_out() {
    let stand = Stand::new(3_130_030_301);
    let (alice, bob) = (&stand.team.alice, &stand.team.bob);
    let laptop = alice.laptop.id;
    let mut device = mls(&alice.laptop);
    uploaded(&alice.session, laptop, &fresh(&mut device, 1), None);
    stand.age("61 days");

    assert_eq!(hand_out_one(&bob.session, laptop), None, "the expired one");
    assert_eq!(
        status(&alice.session, laptop).0,
        0,
        "the expired one is not counted"
    );

    let other = fresh(&mut device, 1);
    let last = last_resort(&mut device);
    uploaded(&alice.session, laptop, &other, Some(&last));
    assert_eq!(hand_out_one(&bob.session, laptop), Some(other[0].clone()));
    assert_eq!(hand_out_one(&bob.session, laptop), Some(last.clone()));
    stand.age("61 days");

    assert_eq!(
        hand_out_one(&bob.session, laptop),
        None,
        "the expired last-resort one"
    );
}
