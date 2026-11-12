//! `skald sign-out` when the server's answer is lost, and on a machine never signed in. Against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind the
//! recording relay, with a test playing the person on the verification page in headless Chrome.

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
use machine::Machine;
use relay::Lose;

const SIGN_OUT: &str = "sign-out";

/// Without the server's answer, sign-out fails and keeps the token, so the machine can retry; the
/// retry, which the server refuses as its session has ended, signs the machine out.
#[test]
fn sign_out_without_an_answer_keeps_the_token_and_a_retry_signs_out() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(4_120_669_001, "ada-lost-sign-out-standin");
    let machine = Machine::new().with_host_name("ada-workstation");
    let signed_in = cloud.signed_in(&browser.page(), &machine, &ada, &["--first-device", "yes"]);
    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.tokens().pop().expect("a session token");
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
