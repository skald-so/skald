//! Long-polls and acks over HTTPS ("Mailbox" and "Receipts"): a long-poll answers the entries
//! after its cursor in `seq` order, at once when it has some or its wait is 0, otherwise with the
//! first to come within its wait, through any instance, and an empty `batch` at the end of its
//! wait, its server's maximum at most. Its cursor acks, as an `ack` does: the server deletes the
//! entries it covers and adds `delivered` for the sender. Expired entries are not answered.

use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Header, ServerStatus};

use crate::common::times::{DAY, EXPIRY, HOUR, RETENTION, WEEK};
use crate::fixture::Fixture;
use crate::https::{
    AT_ONCE, Batch, EARLY, Https, LONG, MAX_WAIT, SLACK, WAKE, answered_within, batch_of,
    stored_through,
};
use crate::link::{Entry, assert_delivered, assert_status, await_seqs, bytes, messages};
use crate::wire::{Socket, assert_nowhere, envelope, move_back, now};

/// The requests of alice's and bob's laptops, and bob's laptop's ID.
fn alice_and_bob(f: &Fixture) -> (Https, Https, DeviceId) {
    let t = &f.team;
    let alice = Https::of(&t.flow, &t.alice);
    (alice, Https::of(&t.flow, &t.bob), t.bob.laptop.id)
}

/// Stores an envelope of a header, one way or another.
type Store<'s> = &'s dyn Fn(&Header, &[u8]);

/// Checks that a long-poll of `https` from `cursor` waiting `wait` seconds answers an empty
/// `batch` once `ends` have passed, and not earlier.
#[track_caller]
fn assert_ends_empty_after(https: &Https, cursor: u64, wait: u64, ends: Duration) {
    let waiting = https.start_poll(cursor, wait);
    let started = waiting.started;
    let (response, at) = waiting.answer();
    let took = at.duration_since(started);
    assert!(
        took + EARLY >= ends && took <= ends + SLACK,
        "a wait of {wait} s that ends after {ends:?} answered after {took:?}"
    );
    assert_eq!(batch_of(&response), Batch::default());
}

/// A long-poll with a wait of 0 answers at once: empty on an empty mailbox, then the entries after
/// its cursor in `seq` order. The next one's cursor deletes what it covers, with the envelopes no
/// other entry refers to.
#[test]
fn a_long_poll_with_a_wait_of_0_answers_at_once_with_the_entries_after_its_cursor() {
    let f = Fixture::new(3_141_410_101);
    let t = &f.team;
    let bob = Https::of(&t.flow, &t.bob);
    let bob_id = t.bob.laptop.id;
    assert_eq!(bob.poll_now(0).entries, []);
    let sent = bytes(&stored_through(
        &Https::of(&t.flow, &t.alice),
        &f,
        bob_id,
        3,
    ));

    assert_eq!(bob.poll_now(0).entries, messages(1, &sent));
    assert_eq!(bob.poll_now(1).entries, messages(2, &sent[1..]));
    await_seqs(f.db(), bob_id, &[2, 3]);
    assert_nowhere(f.db(), &sent[0]);
    assert_eq!(bob.poll_now(3).entries, []);
    await_seqs(f.db(), bob_id, &[]);
    for envelope in &sent {
        assert_nowhere(f.db(), envelope);
    }
}

/// A long-poll with a long wait answers at once when its mailbox holds entries after its cursor.
#[test]
fn a_long_poll_answers_at_once_when_it_has_entries_whatever_its_wait() {
    let f = Fixture::new(3_141_410_111);
    let t = &f.team;
    let bob = Https::of(&t.flow, &t.bob);
    let sent = bytes(&stored_through(
        &Https::of(&t.flow, &t.alice),
        &f,
        t.bob.laptop.id,
        2,
    ));

    let request = bob.poll_request(1, LONG, Some(bob.proof()));
    let asked = Instant::now();
    let response = request.call();
    let took = asked.elapsed();

    assert!(took < AT_ONCE, "answered after {took:?}");
    assert_eq!(batch_of(&response).entries, messages(2, &sent[1..]));
}

/// A waiting long-poll gets an entry stored meanwhile within a second, through the same instance
/// or another, sent over HTTPS or over a WebSocket.
#[test]
fn a_waiting_long_poll_gets_an_entry_stored_meanwhile_within_a_second_through_any_instance() {
    let f = Fixture::new(3_141_410_121);
    let t = &f.team;
    let first = &t.flow.stand.server;
    let other = t.flow.stand.instance(&[], &[]);
    let bob_id = t.bob.laptop.id;
    let bob = Https::of(&t.flow, &t.bob);
    let bob_on_other = Https::to(&other, &t.flow, &t.bob, &t.bob.laptop);
    bob.poll_now(0);
    bob_on_other.poll_now(0);

    // Over HTTPS through the same instance; over a WebSocket through the first instance, to a
    // long-poll on the other; over HTTPS through the other instance, to a long-poll on the first.
    let alice = Https::of(&t.flow, &t.alice);
    let alice_on_other = Https::to(&other, &t.flow, &t.alice, &t.alice.laptop);
    let sends: [(&Https, Store); 3] = [
        (&bob, &|header, sent| {
            alice.store(header, sent);
        }),
        (&bob_on_other, &|header, sent| {
            Socket::to(first, &t.flow, &t.alice, &t.alice.laptop).store(header, sent);
        }),
        (&bob, &|header, sent| {
            alice_on_other.store(header, sent);
        }),
    ];
    for (seq, (receiver, store)) in (1..).zip(sends) {
        let waiting = receiver.start_poll(seq - 1, LONG);
        waiting.assert_waiting();
        let header = f.to(bob_id);
        let sent = envelope(&header);
        store(&header, &sent);
        let batch = answered_within(waiting, Instant::now(), WAKE);
        assert_eq!(batch.entries, [Entry::Message(seq, sent)], "entry {seq}");
    }
}

/// A long-poll that nothing reaches answers an empty `batch` at the end of its wait.
#[test]
fn a_waiting_long_poll_that_nothing_reaches_ends_empty_at_the_end_of_its_wait() {
    let f = Fixture::new(3_141_410_131);
    let bob = Https::of(&f.team.flow, &f.team.bob);
    bob.poll_now(0);

    assert_ends_empty_after(&bob, 0, 3, Duration::from_secs(3));
}

/// On a server whose longest wait is 1 second, a long-poll that asks for 30 ends after 1.
#[test]
fn a_wait_above_the_servers_maximum_ends_at_it() {
    let f = Fixture::with(3_141_410_141, &[MAX_WAIT, "1"]);
    let bob = Https::of(&f.team.flow, &f.team.bob);
    bob.poll_now(0);

    assert_ends_empty_after(&bob, 0, 30, Duration::from_secs(1));
}

/// bob's laptop acks alice's two envelopes, sent with `report` true: the first with `ack`, the
/// second with a long-poll's cursor. Each ack deletes what it covers, and alice's laptop gets
/// `delivered` for it in a long-poll.
#[test]
fn acks_delete_what_they_cover_and_the_sender_gets_delivered() {
    let f = Fixture::new(3_141_410_151);
    let (alice, bob, bob_id) = alice_and_bob(&f);
    let sent = stored_through(&alice, &f, bob_id, 2);
    assert_eq!(bob.poll_now(0).entries, messages(1, &bytes(&sent)));

    // The first with `ack`, leaving the second; the second with a long-poll's cursor.
    let acks: [(&dyn Fn(), &[i64]); 2] = [
        (&|| bob.ack(1), &[2]),
        (&|| assert_eq!(bob.poll_now(2).entries, []), &[]),
    ];
    for ((header, _), (seq, (ack, left))) in sent.iter().zip((1..).zip(acks)) {
        let before = now();
        ack();
        await_seqs(f.db(), bob_id, left);
        let delivered = alice.entries_within(seq - 1, WAKE + SLACK);
        assert_eq!(delivered.len(), 1, "{delivered:?}");
        assert_eq!(assert_delivered(&delivered[0], header, bob_id, before), seq);
    }
}

/// On a server with a mailbox retention of 7 days, of two envelopes to bob, stored 7 days and an
/// hour ago and a day and an hour ago, a long-poll answers only the second; alice's laptop gets
/// `expired` for the first in a long-poll.
#[test]
fn an_expired_entry_is_not_answered_and_its_sender_gets_expired() {
    let week = WEEK.to_string();
    let f = Fixture::with(3_141_410_161, &[RETENTION, &week]);
    let (alice, bob, bob_id) = alice_and_bob(&f);
    let old_header = f.to(bob_id);
    let old = envelope(&old_header);
    let stored = alice.store(&old_header, &old);
    move_back(f.db(), 6 * DAY);
    let new = bytes(&stored_through(&alice, &f, bob_id, 1));
    move_back(f.db(), DAY + HOUR);

    assert_eq!(bob.poll_now(0).entries, messages(2, &new));

    let expired = alice.entries_within(0, EXPIRY);
    assert_eq!(expired.len(), 1, "{expired:?}");
    let at = stored.timestamp - HOUR;
    assert_eq!(
        assert_status(&expired[0], ServerStatus::Expired, &old_header, bob_id, at),
        1
    );
    await_seqs(f.db(), bob_id, &[2]);
    assert_nowhere(f.db(), &old);
}
