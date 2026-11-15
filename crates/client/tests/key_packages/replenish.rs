//! A device keeps its KeyPackages on the server: 20 and a last-resort one from its sign-in; enough
//! for 20 again on a run that connects once fewer than 10 unused ones expire in more than 7 days;
//! a new last-resort one once the last was handed out or expires within 7 days.

use crate::kit::{KEPT, Pair};

/// Once its sign-in ends, Ada's new device has 20 unused KeyPackages and a last-resort one on the
/// server: Bob gets 20 different ones, each of Ada's device, then the last-resort one each time.
/// The first and the last-resort one add Ada's device as the client's own code checks a
/// KeyPackage.
#[test]
fn new_device_uploads_twenty_key_packages_and_a_last_resort_one() {
    let pair = Pair::start(
        [4_130_010_001, 4_130_010_002],
        ["ada-new-kp-standin", "bob-takes-kp-standin"],
    );

    assert_eq!(pair.status(), (KEPT, false), "after the sign-in");
    let (others, last) = pair.take_all();
    pair.assert_adds_ada(&others[0]);
    pair.assert_adds_ada(&last);
    assert_eq!(pair.status(), (0, true), "all handed out");
}

/// With 9 unused KeyPackages left after Bob took 11, Ada's next run that connects uploads 11 new
/// ones: 20 again, none of them one Bob took.
#[test]
fn run_with_nine_left_uploads_eleven() {
    let pair = Pair::start(
        [4_130_010_101, 4_130_010_102],
        ["ada-nine-left-standin", "bob-took-eleven-standin"],
    );
    let taken = pair.take_others(11);
    assert_eq!(pair.status(), (9, false), "Bob took 11");

    pair.recv();

    assert_eq!(pair.status(), (KEPT, false), "after the run");
    let (others, _) = pair.take_all();
    assert!(
        others.iter().all(|one| !taken.contains(one)),
        "a KeyPackage Bob took is handed out again"
    );
}

/// With 10 unused KeyPackages left, Ada's next run uploads none.
#[test]
fn run_with_ten_left_uploads_none() {
    let pair = Pair::start(
        [4_130_010_201, 4_130_010_202],
        ["ada-ten-left-standin", "bob-took-ten-standin"],
    );
    pair.take_others(10);

    pair.recv();

    assert_eq!(pair.status(), (10, false), "after the run");
}

/// With the expiry of 11 of Ada's 20 unused KeyPackages moved within 7 days in the server's
/// database, 9 count; Ada's next run uploads 11, so 20 count again.
#[test]
fn run_uploads_eleven_when_eleven_of_twenty_expire_within_seven_days() {
    let pair = Pair::start(
        [4_130_010_301, 4_130_010_302],
        ["ada-expiring-kp-standin", "bob-watches-kp-standin"],
    );
    pair.expire_soon(11, false);
    assert_eq!(pair.status(), (9, false), "11 expire within 7 days");

    pair.recv();

    assert_eq!(pair.status(), (KEPT, false), "after the run");
}

/// Once Bob took every KeyPackage of Ada's device and then its last-resort one, the server says it
/// is to be replaced; Ada's next run uploads 20 and a new last-resort one, which Bob gets once the
/// 20 are taken.
#[test]
fn handed_out_last_resort_key_package_is_replaced() {
    let pair = Pair::start(
        [4_130_010_401, 4_130_010_402],
        ["ada-used-last-standin", "bob-used-last-standin"],
    );
    let (_, old) = pair.take_all();
    assert_eq!(pair.status(), (0, true), "the last-resort one handed out");

    pair.recv();

    assert_eq!(pair.status(), (KEPT, false), "after the run");
    let (_, new) = pair.take_all();
    assert_ne!(new, old, "the old last-resort one");
}

/// With the expiry of Ada's last-resort KeyPackage moved within 7 days in the server's database,
/// the server says it is to be replaced; Ada's next run replaces it and uploads no other.
#[test]
fn last_resort_key_package_that_expires_within_seven_days_is_replaced() {
    let pair = Pair::start(
        [4_130_010_501, 4_130_010_502],
        ["ada-old-last-standin", "bob-old-last-standin"],
    );
    pair.expire_soon(1, true);
    assert_eq!(
        pair.status(),
        (KEPT, true),
        "the last-resort one expires within 7 days"
    );

    pair.recv();

    assert_eq!(pair.status(), (KEPT, false), "after the run");
}
