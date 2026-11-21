//! A run whose device has no session is not given as waiting, paired or not; once its device signs
//! in to the account and registers there, it is given as waiting, and the run goes on.

use crate::common::devices::{Device, assert_taken};
use crate::kit::{Fresh, pair, register, reply_and_request};
use crate::relay::{Waiting, reply};

#[test]
fn a_run_without_a_session_gives_no_waiting_device_until_its_device_signs_in() {
    let f = Fresh::start(3_175_300_211);
    let issuer = &f.alice.session;
    let tablet = Device::new("Tablet");
    let run = f.open(&tablet);
    assert_eq!(issuer.waiting(), Vec::new(), "unpaired, nothing waits");
    assert_eq!(f.bob.session.waiting(), Vec::new(), "for bob either");

    // The new device's machine signs in to alice's account with the browser and registers it.
    let signed_in = f.alice_signs_in();
    register(&signed_in, &tablet);
    assert_eq!(
        issuer.waiting(),
        vec![Waiting::of(&tablet)],
        "signed in, it waits"
    );

    // The run goes on: alice enters its pairing, and its messages go through.
    assert_eq!(pair(issuer, &run.pairing), run.hello);
    reply_and_request(issuer, &run);
}

#[test]
fn a_paired_run_without_a_session_gives_no_waiting_device() {
    let f = Fresh::start(3_175_300_221);
    let issuer = &f.alice.session;
    let tablet = Device::new("Tablet");
    let run = f.open(&tablet);
    pair(issuer, &run.pairing);
    let reply = reply(&run.session);
    assert_taken(&issuer.send_message(&reply));
    run.messages_with(&reply);
    assert_eq!(issuer.waiting(), Vec::new(), "paired, nothing waits");
    assert_eq!(
        f.alice_signs_in().waiting(),
        Vec::new(),
        "to no session of the account"
    );
}
