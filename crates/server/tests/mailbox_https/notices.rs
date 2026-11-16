//! `space-notice`s over HTTPS ("Frames"; `docs/protocol/membership.md`, "Roster"): a long-poll
//! answers one for each roster revision stored since the device's previous long-poll answer, so
//! that none is lost between requests, and a waiting long-poll gets one at once, through any
//! instance. Only the devices of the accounts that get the space get them.

use std::time::Instant;

use crate::fixture::Fixture;
use crate::https::{Batch, Https, LONG, WAKE, answered_within};

/// A long-poll of bob's laptop waiting on another instance gets the notice of a revision of
/// `acme` within a second.
#[test]
fn a_waiting_long_poll_gets_a_space_notice_through_any_instance() {
    let mut f = Fixture::new(3_141_410_201);
    let t = &f.team;
    let other = t.flow.stand.instance(&[], &[]);
    let bob = Https::to(&other, &t.flow, &t.bob, &t.bob.laptop);
    let space = t.space;
    bob.poll_now(0);
    let waiting = bob.start_poll(0, LONG);
    waiting.assert_waiting();

    f.revise(|_| {});

    let batch = answered_within(waiting, Instant::now(), WAKE);
    let notice = Batch {
        entries: Vec::new(),
        notices: vec![space],
    };
    assert_eq!(batch, notice);
}

/// A revision stored between two long-polls of bob's laptop comes in the second one's answer, and
/// in no later one; eve's laptop, outside `acme`, gets none.
#[test]
fn a_long_poll_gets_a_notice_of_a_revision_stored_since_the_previous_answer() {
    let mut f = Fixture::new(3_141_410_211);
    let bob = Https::of(&f.team.flow, &f.team.bob);
    let eve = Https::of(&f.team.flow, &f.eve);
    let space = f.team.space;
    bob.poll_now(0);
    eve.poll_now(0);

    f.revise(|_| {});

    let notice = Batch {
        entries: Vec::new(),
        notices: vec![space],
    };
    assert_eq!(bob.poll_now(0), notice);
    assert_eq!(bob.poll_now(0), Batch::default(), "the notice again");
    assert_eq!(eve.poll_now(0), Batch::default(), "a notice outside acme");
}
