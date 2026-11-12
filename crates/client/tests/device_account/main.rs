//! The account a device belongs to, when its local state records none, as a state older than that
//! record whose session had no account: its next sign-in records it, so a later sign-in approved
//! by another GitHub user is refused. Against a `skald-server` with GitHub sign-in through the
//! GitHub stand-in, on PostgreSQL, behind the recording relay, with a test playing the person on
//! the verification page in headless Chrome.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use browser::Browser;
use cloud::Cloud;
use machine::{Finished, Machine};

const FIRST_DEVICE: [&str; 2] = ["--first-device", "yes"];

/// Runs `skald sign-out` on `machine`; fails unless it succeeds.
fn sign_out(machine: &Machine) {
    let finished = machine.start(&["sign-out"]).finish();
    assert!(finished.success, "{finished:?}");
}

/// The account the local state of `machine` records for its device.
fn device_account(machine: &Machine) -> Option<String> {
    let db = rusqlite::Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("open the local state");
    db.query_row("SELECT account FROM device", [], |row| row.get(0))
        .expect("the device's account")
}

#[test]
fn device_without_an_account_takes_its_next_sign_ins_and_refuses_another() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(4_120_670_001, "ada-no-account-standin");
    let bob = cloud.user(4_120_670_002, "bob-no-account-standin");
    let machine = Machine::new().with_host_name("ada-workstation");
    let first: Finished = cloud.signed_in(&browser.page(), &machine, &ada, &FIRST_DEVICE);
    assert!(first.success, "{first:?}");
    let account = device_account(&machine).expect("the device's account");
    sign_out(&machine);
    // As a state older than the record whose session had no account.
    rusqlite::Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("open the local state")
        .execute("UPDATE device SET account = NULL", [])
        .expect("forget the device's account");

    let again = cloud.signed_in(&browser.page(), &machine, &ada, &FIRST_DEVICE);
    assert!(again.success, "{again:?}");
    assert_eq!(device_account(&machine), Some(account.clone()));
    sign_out(&machine);

    let refused = cloud.signed_in(&browser.page(), &machine, &bob, &FIRST_DEVICE);
    assert!(!refused.success, "{refused:?}");
    assert!(
        refused.stderr.contains(&format!("GitHub user {},", bob.login)),
        "{refused:?}"
    );
    assert_eq!(device_account(&machine), Some(account));
}
