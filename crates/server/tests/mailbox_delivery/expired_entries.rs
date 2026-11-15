//! What expiry does besides "Mailbox"'s rules: the time `expired` gives is when the entry expired;
//! an idempotency key is forgotten once the retention has passed since its send; the longest `ttl`
//! is stored; a page of entries that all expire as catch-up reaches them does not end it; and an
//! entry that catch-up skips as expired is not acked as delivered.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Header, ServerStatus};

use crate::common::times::{EXPIRY, HOUR, WEEK};
use crate::expiring::{VERSIONS, restart, retained, send_bob_with};
use crate::link::{
    Entry, Link, assert_delivered, assert_status, await_seqs_within, bytes, messages, store_to,
};
use crate::scene::Scene;
use crate::wire::{envelope, move_back, now, query, version};

/// Checks that `entry` is the `expired` of the message of `header` to bob, dated `at`, within the
/// second its time is rounded to.
#[track_caller]
fn assert_expired_at(s: &Scene, entry: &Entry, header: &Header, at: u64) {
    assert_status(entry, ServerStatus::Expired, header, s.bob, at);
    let Entry::Status(_, status) = entry else {
        unreachable!("a status");
    };
    assert!(
        status.timestamp.abs_diff(at) <= 1,
        "expired at {}, not at {at}",
        status.timestamp
    );
}

#[test]
fn expired_gives_when_the_ttl_or_the_retention_ended() {
    let mut s = retained(3_121_250_301);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let (short, stored) = send_bob_with(&mut s, Some(HOUR), true);
    move_back(s.db(), 2 * HOUR);
    let expired = s.alice.entry_within(EXPIRY);
    assert_expired_at(&s, &expired, &short.0, stored.timestamp - HOUR);

    let (kept, stored) = send_bob_with(&mut s, None, true);
    move_back(s.db(), WEEK + HOUR);
    let expired = s.alice.entry_within(EXPIRY);
    assert_expired_at(&s, &expired, &kept.0, stored.timestamp - HOUR);
}

/// The idempotency keys the server keeps.
fn keys(s: &Scene) -> i64 {
    query(s.db(), async |conn| {
        sqlx::query_scalar("SELECT count(*) FROM sends")
            .fetch_one(conn)
            .await
    })
}

#[test]
fn an_idempotency_key_is_forgotten_after_the_retention() {
    let mut s = retained(3_121_250_311);
    let sent = s.send_bob(1, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));
    link.ack(1);
    assert_eq!(keys(&s), 1);

    move_back(s.db(), WEEK + HOUR);
    let deadline = Instant::now() + EXPIRY;
    while keys(&s) > 0 {
        assert!(Instant::now() < deadline, "a key kept after {EXPIRY:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn an_envelope_with_the_longest_ttl_is_stored_and_delivered() {
    let mut s = retained(3_121_250_321);
    let (sent, _) = send_bob_with(&mut s, Some(u64::MAX), true);
    s.bob_from(0, &[Entry::Message(1, sent.1)]);
}

#[test]
fn catch_up_goes_on_past_a_page_of_entries_that_expire_as_it_reaches_them() {
    let mut s = Scene::with(3_121_250_331, &[VERSIONS, "1-2"]);
    // More than a page of version 1, then one of version 2.
    let old = store_to(&mut s.alice, &s.fixture, s.bob, 101, false);
    let t = s.team();
    let mut alice = Link::stating(&t.flow.stand.server, &t.flow, &t.alice, version(2));
    let mut header = s.fixture.to(s.bob);
    header.v = version(2);
    let new = envelope(&header);
    alice.store(&header, &new, false);
    drop(alice);
    assert_eq!(old.len(), 101);

    let server = restart(&mut s, &[VERSIONS, "2-3"], &[]);
    let t = s.team();
    let mut bob = Link::stating(&server, &t.flow, &t.bob, version(3));
    bob.sync(0);
    assert_eq!(bob.catch_up(), [Entry::Message(102, new)]);
    await_seqs_within(s.db(), s.bob, &[102], EXPIRY);
}

#[test]
fn an_ack_right_after_catch_up_past_an_unswept_expired_message_gives_its_sender_expired() {
    let mut s = retained(3_121_250_341);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let (short, stored) = send_bob_with(&mut s, Some(HOUR), true);
    let (kept, _) = send_bob_with(&mut s, None, true);
    move_back(s.db(), 2 * HOUR);

    // Before the server's next look for expired entries.
    let mut link = s.bob_from(0, &[Entry::Message(2, kept.1.clone())]);
    let before = now();
    link.ack(2);
    let expired = s.alice.entry_within(EXPIRY);
    assert_expired_at(&s, &expired, &short.0, stored.timestamp - HOUR);
    assert_delivered(&s.alice.entry(), &kept.0, s.bob, before);
    s.alice.assert_quiet();
}
