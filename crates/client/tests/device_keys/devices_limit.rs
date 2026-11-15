//! The devices an account may have, in the CLI: when the server refuses `skald sign-in`'s device
//! registration with the problem `https://skald.so/problems/devices-limit`, the run fails and says
//! the account `has as many devices as it may`, by the problem's type whatever its `detail`. The
//! relay answers the registration itself with that refusal; the server's own limit is tested in
//! `crates/server/tests/cloud_limits`.

use super::{FIRST_DEVICE, Machine, NO, REGISTER, first_device, ids, says, setup};

/// The server's refusal of a registration over the limit; its `detail` says otherwise than the CLI.
const OVER_LIMIT: &str = r#"{"type":"https://skald.so/problems/devices-limit","title":"Device limit reached","status":403,"detail":"Refused: 50 of 50."}"#;

/// What the CLI says on that refusal.
const SAID: &str = "has as many devices as it may";

/// A second machine of an account whose registration the server refuses as over the limit fails,
/// says the account has as many devices as it may, and leaves the account with its first device.
#[test]
fn registration_over_the_limit_says_the_account_has_as_many_devices_as_it_may() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_122_102_001, "ada-many-devices-standin");
    let machine = Machine::new().with_host_name("ada-first");
    let first = first_device(&cloud, &page, &machine, &ada, &[]);
    cloud
        .relay
        .refuse_with(REGISTER, "403 Forbidden", OVER_LIMIT);
    let other = Machine::new().with_host_name("ada-one-too-many");
    let mark = cloud.relay.mark();

    let refused = cloud.signed_in(&page, &other, &ada, &[FIRST_DEVICE, NO]);

    assert!(!refused.success, "{refused:?}");
    assert!(
        cloud.count_sent(mark, REGISTER) >= 1,
        "no registration sent"
    );
    assert!(
        says(&refused, SAID),
        "it does not say the account has as many devices as it may: {refused:?}"
    );
    assert_eq!(ids(&cloud.devices(&first.token)), vec![first.id]);
}

/// Another refusal of the registration, also a `403`, is not taken for the devices limit: the run
/// fails without saying the account has as many devices as it may.
#[test]
fn another_refusal_of_the_registration_does_not_say_the_devices_limit() {
    let (cloud, browser) = setup();
    let page = browser.page();
    let ada = cloud.user(4_122_102_011, "ada-other-refusal-standin");
    let machine = Machine::new().with_host_name("ada-first");
    first_device(&cloud, &page, &machine, &ada, &[]);
    let other_problem = OVER_LIMIT.replace("devices-limit", "not-allowed");
    cloud
        .relay
        .refuse_with(REGISTER, "403 Forbidden", &other_problem);
    let other = Machine::new().with_host_name("ada-refused");

    let refused = cloud.signed_in(&page, &other, &ada, &[FIRST_DEVICE, NO]);

    assert!(!refused.success, "{refused:?}");
    assert!(
        !says(&refused, SAID),
        "another refusal says the account has as many devices as it may: {refused:?}"
    );
}
