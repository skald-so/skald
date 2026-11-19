//! The `link-notice` over HTTPS and SSE, through any instance: a long-poll answers one when a run
//! opened since the device's previous answer, a waiting one at once, and an event stream sends one;
//! another account's get none.

use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, Frame, Push};

use crate::common::Response;
use crate::common::device_flow::text;
use crate::https::{Https, LONG, WAKE};
use crate::kit::{Link, WAIT};
use crate::sse::{Event, Sse};

/// How long a test watches a stream to see that nothing comes.
const QUIET: Duration = Duration::from_secs(2);

/// Whether the long-poll answer `response`, a `batch`, holds a `link-notice`.
#[track_caller]
fn linking(response: &Response) -> bool {
    assert_eq!(response.status, 200, "{}", text(response));
    let Ok(Frame::Batch(pushes)) = Frame::from_cbor(&response.body) else {
        panic!("not a batch: {:02x?}", response.body);
    };
    pushes.contains(&Push::LinkNotice)
}

/// Whether the next long-poll of `https`, with a wait of 0, answers a `link-notice`.
#[track_caller]
fn polled(https: &Https) -> bool {
    linking(&https.poll_request(0, 0, Some(https.proof())).call())
}

/// alice's and bob's laptops long-poll. Once the machine of alice's desktop opens a run, alice's
/// next long-poll answers a `link-notice` and the one after it none; bob's none. A new hello, which
/// replaces the run, gives alice's next one a `link-notice` again.
#[test]
fn a_long_poll_answers_a_link_notice_once_for_each_run_that_opens() {
    let l = Link::start(3_178_200_001);
    let (alice, bob) = (Https::of(&l.flow, &l.alice), Https::of(&l.flow, &l.bob));
    assert!(!polled(&alice), "alice's, before the hello");
    assert!(!polled(&bob), "bob's, before the hello");

    l.open();

    assert!(polled(&alice), "alice's, after the hello");
    assert!(!polled(&alice), "alice's, the notice again");
    assert!(!polled(&bob), "bob's, after the hello");
    l.open();
    assert!(polled(&alice), "alice's, after a new hello");
}

/// A long-poll of alice's laptop waits on another instance. The machine of alice's desktop opens a
/// run: the long-poll answers a `link-notice` within a second.
#[test]
fn a_waiting_long_poll_on_another_instance_gets_a_link_notice() {
    let l = Link::start(3_178_200_011);
    let other = l.flow.stand.instance(&[], &[]);
    let alice = Https::to(&other, &l.flow, &l.alice, &l.alice.laptop);
    assert!(!polled(&alice), "before the hello");
    let waiting = alice.start_poll(0, LONG);
    waiting.assert_waiting();

    l.open();

    let opened = Instant::now();
    let (response, at) = waiting.answer();
    let took = at.saturating_duration_since(opened);
    assert!(took <= WAKE, "answered {took:?} after the hello");
    assert!(linking(&response), "no link-notice: {:02x?}", response.body);
}

/// alice's and bob's laptops each hold an event stream past catch-up. The machine of alice's
/// desktop opens a run: alice's stream sends a `link-notice`, and bob's nothing.
#[test]
fn an_event_stream_sends_a_link_notice_once_a_run_opens() {
    let l = Link::start(3_178_200_021);
    let mut alice = Sse::of(&l.flow, &l.alice).open(None);
    let mut bob = Sse::of(&l.flow, &l.bob).open(None);
    alice.catch_up();
    bob.catch_up();

    l.open();

    assert_eq!(alice.data_within(WAIT), Event::LinkNotice);
    bob.assert_quiet(QUIET);
}
