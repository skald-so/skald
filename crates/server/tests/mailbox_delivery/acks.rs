//! Acks ("Mailbox"): the server deletes an entry only on its device's ack, every entry up to the
//! cursor; a later `sync` only acks; an envelope goes once no entry refers to it, while its
//! idempotency key stays; a `seq` is never reused.

use std::thread;
use std::time::Duration;

use crate::link::{Entry, QUIET, await_seqs, bytes, messages, seqs};
use crate::scene::Scene;
use crate::wire::{assert_nowhere, envelope};

#[test]
fn after_a_connection_dropped_before_the_ack_the_next_gets_the_same_entries() {
    let mut s = Scene::new(3_121_240_101);
    let mut sent = s.send_bob(3, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));
    let pushed = s.send_bob(1, true);
    assert_eq!(link.entry(), Entry::Message(4, pushed[0].1.clone()));
    sent.extend(pushed);

    drop(link);
    thread::sleep(QUIET);

    // Sent, but not acked: nothing is deleted.
    assert_eq!(seqs(s.db(), s.bob), [1, 2, 3, 4]);
    s.bob_from(0, &messages(1, &bytes(&sent)));
}

#[test]
fn an_ack_deletes_every_entry_up_to_it_and_the_next_connection_gets_none_of_them() {
    let mut s = Scene::new(3_121_240_111);
    let sent = s.send_bob(4, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));

    link.ack(2);
    await_seqs(s.db(), s.bob, &[3, 4]);
    drop(link);

    let mut next = s.bob_from(0, &messages(3, &bytes(&sent[2..])));
    next.ack(4);
    await_seqs(s.db(), s.bob, &[]);
}

#[test]
fn a_later_sync_only_acks_and_starts_no_catch_up() {
    let mut s = Scene::new(3_121_240_121);
    let sent = s.send_bob(3, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));

    link.sync(2);
    link.assert_quiet();
    await_seqs(s.db(), s.bob, &[3]);
    link.sync(0);
    link.assert_quiet();

    // Pushes go on after it, each entry once.
    let more = s.send_bob(1, true);
    assert_eq!(link.entry(), Entry::Message(4, more[0].1.clone()));
    link.assert_quiet();
}

#[test]
fn an_acked_envelope_is_deleted_and_a_retry_of_it_gets_its_first_stored_and_adds_no_entry() {
    let mut s = Scene::new(3_121_240_131);
    let header = s.fixture.to(s.bob);
    let sent = envelope(&header);
    let first = s.alice.store(&header, &sent, true);
    let mut link = s.bob_from(0, &[Entry::Message(1, sent.clone())]);

    link.ack(1);
    await_seqs(s.db(), s.bob, &[]);
    assert_nowhere(s.db(), &sent);

    // Past a second, so that a `stored` made anew would have a later timestamp.
    thread::sleep(Duration::from_millis(1_100));
    let mut again = s.link(&s.team().alice);
    assert_eq!(again.store(&header, &sent, true), first);
    link.assert_quiet();
    assert_eq!(seqs(s.db(), s.bob), Vec::<i64>::new());
    assert_nowhere(s.db(), &sent);
}

#[test]
fn after_the_mailbox_is_emptied_a_new_entry_gets_the_next_seq() {
    let mut s = Scene::new(3_121_240_141);
    let sent = s.send_bob(2, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));
    link.ack(2);
    await_seqs(s.db(), s.bob, &[]);

    let next = s.send_bob(1, true);
    let third = Entry::Message(3, next[0].1.clone());
    assert_eq!(link.entry(), third);
    assert_eq!(seqs(s.db(), s.bob), [3]);
    drop(link);

    s.bob_from(2, &[third]);
}
