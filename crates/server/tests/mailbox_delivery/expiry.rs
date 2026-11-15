//! Expiry by time ("Mailbox"), on a server with a mailbox retention of 7 days: a `message` expires
//! at its storing time plus its envelope's `ttl`, capped by the retention, and a `status` at its
//! adding time plus the retention. The server finds expired entries by itself, also in the mailbox
//! of a device that never connects again, deletes them and never sends them; for an envelope sent
//! with `report` true it adds `expired`, with the recipient device and the time, to the sending
//! device's mailbox ("Receipts"). It keeps each idempotency key at least as long as the retention.

use skald_proto::{MessageId, PROTOCOL_VERSION, ServerStatus};

use crate::expiring::{
    DAY, EXPIRY, HOUR, RETENTION_ENV, WEEK, assert_retry_stores_nothing, move_back, restart,
    retained, send_bob_with,
};
use crate::link::{
    Entry, Link, assert_delivered, assert_status, await_seqs, await_seqs_within, bytes, messages,
    seqs,
};
use crate::scene::Scene;
use crate::wire::{assert_nowhere, now};

#[test]
fn a_message_expires_after_its_ttl_and_is_not_sent() {
    let mut s = retained(3_121_250_001);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let (short, stored) = send_bob_with(&mut s, Some(HOUR), true);
    let (long, _) = send_bob_with(&mut s, Some(3 * HOUR), true);
    // bob's first connection gets both and drops before its ack.
    drop(s.bob_from(0, &messages(1, &bytes(&[short.clone(), long.clone()]))));

    move_back(s.db(), 2 * HOUR);
    let mut link = s.bob_from(0, &[Entry::Message(2, long.1.clone())]);
    let expired = s.alice.entry_within(EXPIRY);
    let at = stored.timestamp - HOUR;
    assert_eq!(
        assert_status(&expired, ServerStatus::Expired, &short.0, s.bob, at),
        1
    );
    await_seqs_within(s.db(), s.bob, &[2], EXPIRY);
    assert_nowhere(s.db(), &short.1);
    s.alice.assert_quiet();

    // The other one is still delivered.
    let before = now();
    link.ack(2);
    assert_delivered(&s.alice.entry(), &long.0, s.bob, before);
    s.alice.assert_quiet();
}

/// Checks that a message to bob with `ttl` expires at the retention: of two, stored 7 days and an
/// hour ago and a day and an hour ago, only the first expires.
fn expires_at_the_retention(github: u64, ttl: Option<u64>) {
    let mut s = retained(github);
    let (old, stored) = send_bob_with(&mut s, ttl, true);
    move_back(s.db(), 6 * DAY);
    let (new, _) = send_bob_with(&mut s, ttl, true);
    move_back(s.db(), DAY + HOUR);

    s.bob_from(0, &[Entry::Message(2, new.1)]);
    await_seqs_within(s.db(), s.bob, &[2], EXPIRY);
    assert_nowhere(s.db(), &old.1);

    // alice connects after `expired` was added.
    await_seqs_within(s.db(), s.team().alice.laptop.id, &[1], EXPIRY);
    s.alice.sync(0);
    let entries = s.alice.catch_up();
    assert_eq!(entries.len(), 1, "one expired: {entries:?}");
    let at = stored.timestamp - HOUR;
    assert_status(&entries[0], ServerStatus::Expired, &old.0, s.bob, at);
    s.alice.assert_quiet();
}

#[test]
fn a_message_without_ttl_expires_at_the_retention() {
    expires_at_the_retention(3_121_250_011, None);
}

#[test]
fn a_message_with_a_ttl_above_the_retention_expires_at_the_retention() {
    expires_at_the_retention(3_121_250_021, Some(30 * DAY));
}

#[test]
fn the_sender_gets_expired_for_report_true_only_also_when_the_recipient_never_connects() {
    let mut s = retained(3_121_250_031);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    // When they expire, once moved back: an hour before they were stored.
    let at = now() - HOUR;
    let mut sent = s.send_bob(1, true);
    sent.extend(s.send_bob(1, false));
    sent.extend(s.send_bob(1, true));
    move_back(s.db(), WEEK + HOUR);

    // bob does not connect: the server finds the entries by itself.
    let statuses = [s.alice.entry_within(EXPIRY), s.alice.entry_within(EXPIRY)];
    let mut reported: Vec<MessageId> = Vec::new();
    for status in &statuses {
        let Entry::Status(_, found) = status else {
            panic!("a status, not {status:?}");
        };
        let (header, _) = sent
            .iter()
            .find(|(header, _)| header.msg_id == found.msg_id)
            .unwrap_or_else(|| panic!("a status of a message alice sent: {found:?}"));
        assert_status(status, ServerStatus::Expired, header, s.bob, at);
        reported.push(found.msg_id);
    }
    assert!(
        reported.contains(&sent[0].0.msg_id) && reported.contains(&sent[2].0.msg_id),
        "one expired for each message sent with report true: {statuses:?}"
    );
    let mut added: Vec<u64> = statuses.iter().map(Entry::seq).collect();
    added.sort_unstable();
    assert_eq!(added, [1, 2]);
    s.alice.assert_quiet();

    await_seqs_within(s.db(), s.bob, &[], EXPIRY);
    for (_, envelope) in &sent {
        assert_nowhere(s.db(), envelope);
    }
    s.bob_from(0, &[]);
    s.alice.assert_quiet();
}

#[test]
fn a_status_expires_at_the_retention() {
    let mut s = retained(3_121_250_041);
    let alice = s.team().alice.laptop.id;
    let first = s.send_bob(1, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&first)));
    link.ack(1);
    await_seqs(s.db(), alice, &[1]);
    move_back(s.db(), 6 * DAY);
    let second = s.send_bob(1, true);
    assert_eq!(link.entry(), Entry::Message(2, second[0].1.clone()));
    link.ack(2);
    await_seqs(s.db(), alice, &[1, 2]);

    move_back(s.db(), DAY + HOUR);
    await_seqs_within(s.db(), alice, &[2], EXPIRY);

    s.alice.sync(0);
    let entries = s.alice.catch_up();
    let [Entry::Status(2, status)] = entries.as_slice() else {
        panic!("the later delivered alone, not {entries:?}");
    };
    // Its time moved back with the rest.
    let found = (status.status, status.msg_id, status.device);
    assert_eq!(found, (ServerStatus::Delivered, second[0].0.msg_id, s.bob));
    // An expired status reports nothing.
    link.assert_quiet();
    assert_eq!(seqs(s.db(), s.bob), Vec::<i64>::new());
    s.alice.assert_quiet();
}

#[test]
fn an_idempotency_key_is_kept_for_the_retention() {
    let mut s = retained(3_121_250_051);
    let (sent, first) = send_bob_with(&mut s, None, true);
    let mut link = s.bob_from(0, &[Entry::Message(1, sent.1.clone())]);
    link.ack(1);
    await_seqs(s.db(), s.bob, &[]);
    assert_nowhere(s.db(), &sent.1);

    move_back(s.db(), WEEK - HOUR);
    assert_retry_stores_nothing(&s, &sent, &first, &mut link);
}

#[test]
fn the_retention_also_comes_from_the_environment() {
    let mut s = Scene::new(3_121_250_061);
    let sent = s.send_bob(1, true);
    let server = restart(&mut s, &[], &[(RETENTION_ENV, &WEEK.to_string())]);
    move_back(s.db(), WEEK + HOUR);

    let t = s.team();
    let mut bob = Link::stating(&server, &t.flow, &t.bob, PROTOCOL_VERSION);
    bob.sync(0);
    assert_eq!(bob.catch_up(), []);
    await_seqs_within(s.db(), s.bob, &[], EXPIRY);
    assert_nowhere(s.db(), &sent[0].1);
}
