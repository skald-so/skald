//! A run ends on a `link-abort` from either side, once its device has a certificate, and at the
//! code's expiry, after which its later messages are refused and its device is no longer given as
//! waiting; a device's new hello replaces its open run, whose messages are then refused.

use std::time::{Duration, Instant};

use crate::common::devices::assert_taken;
use crate::kit::{
    LIFETIME, Link, Waiting, abort, assert_refused_link, grant, hello, reply, request,
};

/// Checks that the run `session` of alice's desktop has ended: its device is not waiting, a later
/// message from either side is refused and the run's messages hold none of them.
#[track_caller]
fn assert_ended(l: &Link, session: &[u8; 16], what: &str) {
    l.alice.session.until_not_waiting(l.desktop());
    assert_eq!(l.new_side.waiting(), Vec::new(), "{what}: nothing waits");
    assert_refused_link(
        &l.alice.session.send_message(&grant(session)),
        &format!("{what}: a grant"),
    );
    assert_refused_link(
        &l.new_side.send_message(&request(session)),
        &format!("{what}: a request"),
    );
    assert_refused_link(
        &l.alice.session.send_message(&abort(session)),
        &format!("{what}: an abort"),
    );
}

#[test]
fn a_link_abort_from_the_new_device_ends_the_run() {
    let l = Link::start(3_170_300_111);
    let (session, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&session)));
    assert_taken(&l.new_side.send_message(&abort(&session)));
    assert_ended(&l, &session, "aborted by the new device");

    // The device may open a new run.
    let (again, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&again)));
}

#[test]
fn a_link_abort_from_the_issuing_device_ends_the_run() {
    let l = Link::start(3_170_300_121);
    let (session, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&session)));
    assert_taken(&l.new_side.send_message(&request(&session)));
    let abort = abort(&session);
    assert_taken(&l.alice.session.send_message(&abort));
    assert_ended(&l, &session, "aborted by the issuing device");
}

#[test]
fn a_run_ends_once_its_device_has_a_certificate() {
    let l = Link::start(3_170_300_131);
    let (session, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&session)));
    l.certify_desktop();
    assert_ended(&l, &session, "certified");
    let session = crate::kit::fresh_session();
    assert_refused_link(
        &l.new_side.send_hello(l.desktop(), &hello(&session)),
        "a hello naming the certified device",
    );
    assert_eq!(l.alice.session.waiting(), Vec::new());
}

#[test]
fn a_run_ends_at_the_codes_expiry() {
    let lifetime = Duration::from_secs(4);
    let l = Link::with(3_170_300_141, &[LIFETIME, &lifetime.as_secs().to_string()]);
    let started = Instant::now();
    let (session, _) = l.open();
    let replied = l.alice.session.send_message(&reply(&session));
    let waiting = l.alice.session.waiting();
    // Only a run whose reply and listing came well within the lifetime must still have been open.
    if started.elapsed() < lifetime / 2 {
        assert_taken(&replied);
        assert_eq!(
            waiting,
            vec![Waiting::of(&l.alice.uncertified)],
            "open before its expiry"
        );
    }
    assert_ended(&l, &session, "expired");
    assert!(
        started.elapsed() >= lifetime,
        "ended {:?} after the hello, before its lifetime of {lifetime:?}",
        started.elapsed()
    );
}

#[test]
fn a_devices_new_hello_replaces_its_run() {
    let l = Link::start(3_170_300_151);
    let (first, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&first)));
    let (second, latest) = l.open();
    assert_eq!(l.alice.session.hello(), Some(latest));
    assert_eq!(
        l.alice.session.waiting(),
        vec![Waiting::of(&l.alice.uncertified)],
        "the device is given once"
    );
    assert_refused_link(
        &l.new_side.send_message(&request(&first)),
        "a request of the replaced run",
    );
    assert_refused_link(
        &l.alice.session.send_message(&grant(&first)),
        "a grant of the replaced run",
    );
    let reply = reply(&second);
    assert_taken(&l.alice.session.send_message(&reply));
    l.new_side.messages_with(&second, &reply);
}
