//! A helper that dies, and helpers started at once: one helper runs per local state, and a dead
//! one loses nothing.

use std::time::{Duration, Instant};

use crate::fixtures::{SEND, await_running};
use crate::front::Answers;
use crate::gaps::{bob_in, bob_sends};
use crate::helped::{HELPER, Helped, bob_reads, timeout_of, times};
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// What Ada's machine sends and Bob's device says; none is anywhere else.
const FIRST: &str = "helper test: the database migration starts";
const SECOND: &str = "helper test: the database migration is done";
const BOB_SAYS: &str = "helper test: the replicas caught up";

/// The request timeout of the command that waits while its helper is killed, in seconds: longer
/// than the test waits for it to fail.
const WAITING_TIMEOUT: &str = "60";
/// How soon after its helper is killed the waiting command fails.
const FAILS_WITHIN: Duration = Duration::from_secs(20);

/// Ada's first `skald send` leaves a helper running, and Bob's device sends a message to the
/// channel. The front then loses the server's `stored` answers: a second `skald send` waits for
/// one through the helper, which is killed. That command fails well before its timeout. With the
/// answers forwarded again, the next `skald recv` starts a new helper and shows Bob's message, and
/// the one after shows nothing again; Bob's device reads Ada's two messages, each once, with the
/// counters 1 and 2.
#[test]
fn helper_killed_while_a_command_waits_fails_it_and_the_next_command_loses_nothing() {
    let mut talk = Talk::new(
        &[4_146_031_001, 4_146_031_002],
        &["ada-loses-her-helper", "bob-talks-to-dead-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let (from_bob, channel, _) = bob_in(&talk);
    helped.sent(&talk, FIRST);
    let helper = helped.one_helper();
    bob_sends(&mut talk, 1, BOB_SAYS);
    talk.front.answer(Answers::Lose);
    let lost = talk.front.lost();
    let channel_text = channel.to_string();
    let mut waiting = helped.start(&timeout_of(WAITING_TIMEOUT), &[SEND, &channel_text, SECOND]);
    await_running(&mut waiting, "lost stored answer", || {
        talk.front.lost() > lost
    });

    helped.kill(helper);

    let killed = Instant::now();
    let failed = waiting.finish();
    assert!(!failed.success, "it outlived its helper: {failed:?}");
    assert!(
        killed.elapsed() < FAILS_WITHIN,
        "it failed {:?} after its helper was killed: {failed:?}",
        killed.elapsed()
    );
    talk.front.answer(Answers::Forward);
    let next = helped.received();
    let restarted = helped.one_helper();
    assert_ne!(restarted, helper, "the killed helper still runs");
    let again = helped.received();
    let shown = format!("{}{}", next.shown(), again.shown());
    assert_shows_once(&next.shown(), BOB_SAYS, (channel, &from_bob, BOB_HOST));
    assert_eq!(times(&shown, BOB_SAYS), 1, "shown twice: {shown}");
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// Two `skald helper` start at once on Ada's machine: soon one helper runs, and keeps running.
/// Ada's `skald send` then goes through it, starting no other, and Bob's device reads the message.
#[test]
fn of_two_helpers_started_at_once_one_keeps_running() {
    let mut talk = Talk::new(
        &[4_146_032_001, 4_146_032_002],
        &["ada-starts-two-helpers", "bob-reads-one-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let _first = helped.start(&[], &[HELPER]);
    let _second = helped.start(&[], &[HELPER]);

    let survivor = helped.await_one_steady(Duration::from_secs(3));

    helped.sent(&talk, FIRST);
    assert_eq!(
        helped.helpers(),
        [survivor],
        "not the helper that kept running"
    );
    bob_reads(&mut talk, &[FIRST]);
}
