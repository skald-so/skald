//! `skald sign-out` when the server's answer is lost, and on a machine never signed in; and the
//! account a device belongs to when its local state records none, as a state older than that
//! record whose session had no account. Against a `skald-server` with GitHub sign-in through the
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

use std::path::PathBuf;

use browser::Browser;
use cloud::Cloud;
use machine::{FIRST_DEVICE, Machine, SIGN_OUT, YES};
use relay::Lose;

/// Without the server's answer, sign-out fails and keeps the token, so the machine can retry; the
/// retry, which the server refuses as its session has ended, signs the machine out.
#[test]
fn sign_out_without_an_answer_keeps_the_token_and_a_retry_signs_out() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(4_120_669_001, "ada-lost-sign-out-standin");
    let (machine, signed_in) = cloud.first_machine(&browser.page(), &ada, "ada-workstation");
    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.last_token();
    assert_eq!(machine.files_holding(&token).len(), 1);

    cloud.relay.lose_answer("DELETE /session ", Lose::Close);
    let lost = machine.start(&[SIGN_OUT]).finish();
    assert!(!lost.success, "{lost:?}");
    assert!(lost.shown().contains("still signed in"), "{lost:?}");
    assert_eq!(cloud.relay.lost(), 1);
    assert_eq!(machine.files_holding(&token).len(), 1, "the token is gone");

    let retried = machine.start(&[SIGN_OUT]).finish();
    assert!(retried.success, "{retried:?}");
    assert!(retried.shown().contains("Signed out"), "{retried:?}");
    assert_eq!(machine.files_holding(&token), Vec::<PathBuf>::new());
    let (status, body) = cloud.session_of(&token);
    assert_eq!(status, 401, "{body}");
}

/// On a machine that never signed in, sign-out says so, sends nothing and leaves no state behind.
#[test]
fn sign_out_never_signed_in_says_so() {
    let cloud = Cloud::start(120, 1);
    let machine = Machine::new();
    let since = cloud.relay.mark();

    let finished = machine.start(&[SIGN_OUT]).finish();

    assert!(finished.success, "{finished:?}");
    assert!(finished.shown().contains("not signed in"), "{finished:?}");
    assert!(cloud.relay.sent_since(since).is_empty());
    assert_eq!(machine.files(), Vec::<PathBuf>::new());
}

/// The arguments that sign the account's first device in.
const FIRST: [&str; 2] = [FIRST_DEVICE, YES];

/// Runs `skald sign-out` on `machine`; fails unless it succeeds.
fn sign_out(machine: &Machine) {
    let finished = machine.start(&[SIGN_OUT]).finish();
    assert!(finished.success, "{finished:?}");
}

/// The account the local state of `machine` records for its device.
fn device_account(machine: &Machine) -> Option<String> {
    let db = rusqlite::Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("open the local state");
    db.query_row("SELECT account FROM device", [], |row| row.get(0))
        .expect("the device's account")
}

/// A device whose local state records no account takes its next sign-in's account, so a later
/// sign-in approved by another GitHub user is refused.
#[test]
fn device_without_an_account_takes_its_next_sign_ins_and_refuses_another() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(4_120_670_001, "ada-no-account-standin");
    let bob = cloud.user(4_120_670_002, "bob-no-account-standin");
    let (machine, first) = cloud.first_machine(&browser.page(), &ada, "ada-workstation");
    assert!(first.success, "{first:?}");
    let account = device_account(&machine).expect("the device's account");
    sign_out(&machine);
    // As a state older than the record whose session had no account.
    rusqlite::Connection::open(machine.state_dir().join("state.sqlite3"))
        .expect("open the local state")
        .execute("UPDATE device SET account = NULL", [])
        .expect("forget the device's account");

    let again = cloud.signed_in(&browser.page(), &machine, &ada, &FIRST);
    assert!(again.success, "{again:?}");
    assert_eq!(device_account(&machine), Some(account.clone()));
    sign_out(&machine);

    let refused = cloud.signed_in(&browser.page(), &machine, &bob, &FIRST);
    assert!(!refused.success, "{refused:?}");
    assert!(
        refused
            .stderr
            .contains(&format!("account {}, which", bob.login)),
        "{refused:?}"
    );
    assert_eq!(device_account(&machine), Some(account));
}
