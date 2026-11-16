//! Runs of `skald send` on one local state: one at a time, nothing lost, and a dead run's lock
//! freed at once.

use std::time::{Duration, Instant};

use crate::fixtures::SEND;
use crate::held::{
    LONG, SHORT, assert_failed_on_the_lock, assert_waits, holding, release, send_holding, send_of,
    sent_ids, start_send,
};
use crate::kit::received;
use crate::talk::Talk;

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "lock test: the release branch is frozen";
const SECOND: &str = "lock test: the release branch is open again";
const THIRD: &str = "lock test: nobody merges until noon";

/// The request timeout of the run after a killed one, in seconds: a lock its dead holder left
/// would keep it out this long.
const AFTER_KILLED: u64 = 20;

/// Two `skald send` in the channel at once on Ada's machine: while the first waits for the server
/// with the lock held, the second waits for the lock and reaches no server. Once the server
/// answers, both succeed; the front saw two message envelopes, and Bob's device reads both, the
/// first with the counter 1 and the second with 2, in that order.
#[test]
fn two_sends_at_once_store_both_messages_with_counters_in_order() {
    let mut talk = Talk::new(
        &[4_145_051_001, 4_145_051_002],
        &["ada-sends-twice-at-once", "bob-reads-both-sends"],
        false,
    );
    let from = talk.front.sent().len();
    let first = send_holding(&talk, FIRST);
    let mut second = start_send(&talk, LONG, SECOND);

    assert_waits(&talk, &mut second);
    release(&talk, vec![first, second]);

    let sent = sent_ids(&talk, from);
    assert_eq!(sent.len(), 2, "not two message envelopes: {sent:?}");
    talk.bob_reads_from_ada(&[(sent[0], 1, FIRST), (sent[1], 2, SECOND)]);
}

/// A `skald send` on Ada's machine waits for the server with the lock held. Another, with a
/// timeout of two seconds, fails after it with a message naming the lock, while the first still
/// runs. The first then succeeds; neither then nor after Ada's next `skald recv`, which sends what
/// waits in the outbox, does the front see the failed run's message: Bob's device reads only the
/// first, with the counter 1.
#[test]
fn send_waiting_for_the_lock_past_its_timeout_fails_naming_it_and_sends_nothing() {
    let mut talk = Talk::new(
        &[4_145_052_001, 4_145_052_002],
        &["ada-waits-too-long", "bob-reads-one-send"],
        false,
    );
    let from = talk.front.sent().len();
    let mut first = send_holding(&talk, FIRST);
    let started = Instant::now();

    let gave_up = start_send(&talk, SHORT, SECOND).finish();

    assert_failed_on_the_lock(&gave_up);
    assert!(
        started.elapsed() >= Duration::from_secs(SHORT),
        "it failed before its timeout: {gave_up:?}"
    );
    first.assert_running();
    release(&talk, vec![first]);
    received(&talk.ada.machine);
    let sent = sent_ids(&talk, from);
    assert_eq!(sent.len(), 1, "not one message envelope: {sent:?}");
    talk.bob_reads_from_ada(&[(sent[0], 1, FIRST)]);
}

/// A `skald send` on Ada's machine waits for the server with the lock held, and is killed. The
/// next `skald send`, with a timeout of [`AFTER_KILLED`], reaches the server well within it, and
/// holds the lock from then on: a third, with a timeout of two seconds, fails naming the lock.
/// Once the server answers, the second succeeds: Bob's device reads only its message, with the
/// counter 1.
#[test]
fn send_killed_while_holding_the_lock_lets_the_next_in_at_once() {
    let mut talk = Talk::new(
        &[4_145_053_001, 4_145_053_002],
        &["ada-killed-holding-lock", "bob-reads-after-kill"],
        false,
    );
    let from = talk.front.sent().len();
    send_holding(&talk, FIRST).kill();

    let [channel, text] = send_of(&talk, SECOND);
    let (next, took) = holding(&talk, AFTER_KILLED, &[SEND, &channel, &text]);

    assert!(
        took < Duration::from_secs(AFTER_KILLED / 2),
        "the next run got in after {took:?}"
    );
    let third = start_send(&talk, SHORT, THIRD).finish();
    assert_failed_on_the_lock(&third);
    release(&talk, vec![next]);
    let sent = sent_ids(&talk, from);
    assert_eq!(sent.len(), 1, "not one message envelope: {sent:?}");
    talk.bob_reads_from_ada(&[(sent[0], 1, SECOND)]);
}
