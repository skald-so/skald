//! A run between two sessions of an account goes through byte for byte, also with each side on
//! another instance; the account's latest hello is given; a session of another account gets no
//! hello or message of the run, and relays none into it.

use crate::common::devices::assert_taken;
use crate::kit::{
    Link, Waiting, abort, assert_refused_link, grant, hello, position, reply, request,
};

#[test]
fn a_run_between_two_sessions_of_an_account_goes_through_byte_for_byte() {
    let l = Link::start(3_170_300_011);
    let issuer = &l.alice.session;
    let (session, sent) = l.open();
    assert_eq!(
        issuer.hello(),
        Some(sent),
        "the issuing device takes the hello"
    );

    let reply = reply(&session);
    assert_taken(&issuer.send_message(&reply));
    l.new_side.messages_with(&session, &reply);

    let request = request(&session);
    assert_taken(&l.new_side.send_message(&request));
    issuer.messages_with(&session, &request);

    let grant = grant(&session);
    assert_taken(&issuer.send_message(&grant));
    let got = l.new_side.messages_with(&session, &grant);
    assert!(
        position(&got, &reply) < position(&got, &grant),
        "in the order taken: {got:?}"
    );
}

#[test]
fn a_run_goes_through_with_each_side_on_another_instance() {
    let l = Link::start(3_170_300_021);
    let other = l.flow.stand.instance(&[], &[]);
    let new_side = &l.new_side;
    let issuer = l.alice.session.on(&other);

    let (session, sent) = l.open();
    assert_eq!(
        issuer.hello(),
        Some(sent),
        "the hello on the other instance"
    );
    assert_eq!(
        issuer.waiting(),
        vec![Waiting::of(&l.alice.uncertified)],
        "the waiting device on the other instance"
    );

    let reply = reply(&session);
    assert_taken(&issuer.send_message(&reply));
    new_side.messages_with(&session, &reply);
    let request = request(&session);
    assert_taken(&new_side.send_message(&request));
    issuer.messages_with(&session, &request);
    let grant = grant(&session);
    assert_taken(&issuer.send_message(&grant));
    new_side.messages_with(&session, &grant);

    // A `link-abort` sent on one instance ends the run on the other.
    assert_taken(&new_side.send_message(&abort(&session)));
    issuer.until_not_waiting(l.desktop());
    assert_refused_link(
        &issuer.send_message(&crate::kit::grant(&session)),
        "a grant after the abort",
    );
}

#[test]
fn the_accounts_latest_hello_is_given() {
    let l = Link::start(3_170_300_031);
    let tablet = l.register("Tablet");
    let (_, first) = l.open();
    assert_eq!(l.alice.session.hello(), Some(first));

    let session = crate::kit::fresh_session();
    let second = hello(&session);
    assert_taken(&l.new_side.send_hello(tablet.id, &second));
    assert_eq!(l.alice.session.hello(), Some(second.clone()), "the latest");
    assert_eq!(
        l.new_side.hello(),
        Some(second),
        "to each session of the account"
    );
    let mut both = vec![Waiting::of(&l.alice.uncertified), Waiting::of(&tablet)];
    both.sort();
    assert_eq!(l.alice.session.waiting(), both);
}

#[test]
fn a_session_of_another_account_gets_no_hello_or_message_of_the_run() {
    let l = Link::start(3_170_300_041);
    let bob = &l.bob.session;
    let issuer = &l.alice.session;
    let (session, _) = l.open();
    let reply = reply(&session);
    assert_taken(&issuer.send_message(&reply));
    let request = request(&session);
    assert_taken(&l.new_side.send_message(&request));
    issuer.messages_with(&session, &request);

    assert_eq!(bob.hello(), None, "bob's account has no hello");
    assert_eq!(bob.waiting(), Vec::new(), "nor a waiting device");
    let got = bob.messages_response(&session);
    if got.status == 200 {
        assert_eq!(
            crate::kit::messages_of(&got),
            Vec::<Vec<u8>>::new(),
            "bob gets none"
        );
    } else {
        assert_refused_link(&got, "bob's request for the run's messages");
    }

    // Nor does bob's session relay into it.
    let bobs = grant(&session);
    assert_refused_link(&bob.send_message(&bobs), "bob's grant");
    assert_refused_link(&bob.send_message(&abort(&session)), "bob's abort");
    let ours = grant(&session);
    assert_taken(&issuer.send_message(&ours));
    let got = l.new_side.messages_with(&session, &ours);
    assert!(!got.contains(&bobs), "bob's grant is not relayed: {got:?}");
    assert_eq!(
        l.alice.session.waiting(),
        vec![Waiting::of(&l.alice.uncertified)],
        "bob's abort ends nothing"
    );

    // bob's own run is apart from alice's.
    let bobs_session = crate::kit::fresh_session();
    let bobs_hello = hello(&bobs_session);
    assert_taken(&bob.send_hello(l.bob.uncertified.id, &bobs_hello));
    assert_eq!(bob.hello(), Some(bobs_hello));
    assert_eq!(bob.waiting(), vec![Waiting::of(&l.bob.uncertified)]);
    assert_eq!(
        issuer.waiting(),
        vec![Waiting::of(&l.alice.uncertified)],
        "alice's sessions get only her device"
    );
    assert_ne!(
        issuer.hello(),
        bob.hello(),
        "alice's sessions get only her hello"
    );
}
