//! What the server keeps of runs: a `session` names one run of an account, so a hello with the
//! `session` of another device's open run is refused; a run is closed at once at its expiry or once
//! its device has a certificate, before the server deletes it with its messages. A hello sent as a
//! later message and a `session` path that is not 16 bytes are refused.

use std::thread;
use std::time::{Duration, Instant};

use crate::common::device_flow::{json, text};
use crate::common::devices::{assert_taken, b64};
use crate::kit::{
    LIFETIME, Link, MESSAGES, WAIT, Waiting, assert_refused_link, hello, reply, request,
};

/// The rows of runs and later messages the database holds.
fn kept(l: &Link) -> Vec<String> {
    (l.flow.stand.db.rows().into_iter())
        .filter(|row| row.starts_with("link_runs: ") || row.starts_with("link_messages: "))
        .collect()
}

/// Checks that the database holds a run and a later message.
#[track_caller]
fn assert_kept(l: &Link) {
    let kept = kept(l);
    for table in ["link_runs: ", "link_messages: "] {
        assert!(kept.iter().any(|row| row.starts_with(table)), "{kept:?}");
    }
}

/// Waits until the database holds no run and no later message, which it must within [`WAIT`].
#[track_caller]
fn until_no_run_is_kept(l: &Link) {
    let deadline = Instant::now() + WAIT;
    loop {
        let kept = kept(l);
        if kept.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the database keeps {kept:?} after {WAIT:?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

#[test]
fn a_hello_with_the_session_of_another_devices_run_is_refused() {
    let l = Link::start(3_170_300_411);
    let tablet = l.register("Tablet");
    let (session, sent) = l.open();
    assert_refused_link(
        &l.new_side.send_hello(tablet.id, &hello(&session)),
        "the tablet's hello with the desktop's session",
    );
    assert_eq!(l.alice.session.hello(), Some(sent));
    assert_eq!(
        l.alice.session.waiting(),
        vec![Waiting::of(&l.alice.uncertified)]
    );
    let request = request(&session);
    assert_taken(&l.new_side.send_message(&request));
    assert_eq!(
        l.alice.session.messages_with(&session, &request),
        vec![request]
    );
}

#[test]
fn a_run_that_ended_at_its_expiry_is_deleted() {
    let lifetime = Duration::from_secs(4);
    let l = Link::with(3_170_300_421, &[LIFETIME, &lifetime.as_secs().to_string()]);
    let started = Instant::now();
    let (session, _) = l.open();
    let requested = l.new_side.send_message(&request(&session));
    let kept = kept(&l);
    // Only a run whose request and rows came well within the lifetime must still have been kept.
    if started.elapsed() < lifetime / 2 {
        assert_taken(&requested);
        assert_eq!(kept.len(), 2, "the run and its request: {kept:?}");
    }
    until_no_run_is_kept(&l);
}

#[test]
fn a_run_whose_device_has_a_certificate_is_deleted() {
    let l = Link::start(3_170_300_431);
    let (session, _) = l.open();
    assert_taken(&l.alice.session.send_message(&reply(&session)));
    assert_kept(&l);
    l.certify_desktop();
    until_no_run_is_kept(&l);
}

/// Checks that `response` is a `400` with the problem type `kind`; `what` names the case.
#[track_caller]
fn assert_bad(response: &crate::common::Response, kind: &str, what: &str) {
    assert_eq!(response.status, 400, "{what}: {}", text(response));
    assert_eq!(
        json(response)["type"],
        format!("https://skald.so/problems/{kind}"),
        "{what}"
    );
}

/// Checks, without waiting for the server's sweep, that alice's desktop no longer waits and that its
/// run with `session` takes and gives no message; `what` names the case.
#[track_caller]
fn assert_closed(l: &Link, session: &[u8; 16], what: &str) {
    assert_eq!(l.alice.session.waiting(), vec![], "{what}");
    assert_eq!(l.alice.session.hello(), None, "{what}");
    assert_refused_link(&l.alice.session.send_message(&reply(session)), what);
    assert_refused_link(&l.new_side.messages_response(session), what);
}

#[test]
fn a_run_is_closed_right_after_its_device_gets_a_certificate() {
    let l = Link::start(3_170_300_441);
    let (session, _) = l.open();
    l.certify_desktop();
    assert_closed(&l, &session, "right after the certificate");
}

#[test]
fn a_run_is_closed_right_after_its_lifetime() {
    let l = Link::with(3_170_300_451, &[LIFETIME, "1"]);
    let (session, _) = l.open();
    thread::sleep(Duration::from_millis(1_200));
    assert_closed(&l, &session, "right after the lifetime");
}

#[test]
fn a_hello_sent_as_a_later_message_is_refused_and_not_relayed() {
    let l = Link::start(3_170_300_461);
    let (session, sent) = l.open();
    assert_bad(
        &l.new_side.send_message(&hello(&session)),
        "invalid-link-message",
        "a hello to the messages path",
    );
    assert_eq!(l.alice.session.messages(&session), Vec::<Vec<u8>>::new());
    assert_eq!(l.alice.session.hello(), Some(sent));
}

#[test]
fn a_session_path_not_of_16_bytes_is_refused() {
    let l = Link::start(3_170_300_471);
    for len in [15, 17] {
        let path = format!("{MESSAGES}/{}", b64(&vec![7; len]));
        assert_bad(
            &l.alice.session.call("get", &path, None),
            "invalid-request",
            &format!("a session of {len} bytes"),
        );
    }
}
