//! Catch-up and push ("Mailbox"): nothing before a connection's first `sync`; the first `sync`
//! starts catch-up after its cursor, in `seq` order, up to an empty `batch`; then each new entry
//! comes as it is added.

use std::thread;

use crate::link::{Entry, await_seqs, bytes, messages, store_to};
use crate::scene::Scene;
use crate::wire::envelopes_of;

#[test]
fn a_device_that_connects_after_the_sends_gets_them_in_order_then_an_empty_batch() {
    let mut s = Scene::new(3_121_240_001);
    let mut carol = s.link(&s.team().carol);
    let mut sent = s.send_bob(2, true);
    sent.extend(store_to(&mut carol, &s.fixture, s.bob, 1, true));
    sent.extend(s.send_bob(2, false));

    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));

    // Then each new entry at once.
    for seq in 6..=8 {
        let more = s.send_bob(1, true);
        assert_eq!(link.entry(), messages(seq, &bytes(&more))[0]);
    }
    link.assert_quiet();
}

#[test]
fn catch_up_of_an_empty_mailbox_ends_at_once() {
    let mut s = Scene::new(3_121_240_011);
    let mut link = s.bob_from(0, &[]);

    let sent = s.send_bob(1, true);
    assert_eq!(link.entry(), Entry::Message(1, sent[0].1.clone()));
    link.assert_quiet();
}

#[test]
fn nothing_comes_before_the_first_sync() {
    let mut s = Scene::new(3_121_240_021);
    let mut sent = s.send_bob(1, true);

    let mut link = s.link(&s.team().bob);
    link.assert_quiet();
    sent.extend(s.send_bob(1, true));
    link.assert_quiet();
    // A send before the first `sync` is answered, and still nothing comes.
    store_to(&mut link, &s.fixture, s.team().carol.laptop.id, 1, true);
    link.assert_quiet();

    link.sync(0);
    assert_eq!(link.catch_up(), messages(1, &bytes(&sent)));
}

#[test]
fn catch_up_starts_after_the_cursor_of_the_first_sync_which_acks() {
    let mut s = Scene::new(3_121_240_031);
    let sent = s.send_bob(4, true);

    s.bob_from(2, &messages(3, &bytes(&sent[2..])));

    await_seqs(s.db(), s.bob, &[3, 4]);
}

#[test]
fn entries_added_during_catch_up_come_once_in_seq_order() {
    let mut s = Scene::new(3_121_240_041);
    let mut carol = s.link(&s.team().carol);
    s.send_bob(20, true);
    let mut link = s.link(&s.team().bob);

    let mut got = thread::scope(|scope| {
        let (fixture, bob) = (&s.fixture, s.bob);
        scope.spawn(move || store_to(&mut carol, fixture, bob, 20, true));
        link.sync(0);
        link.catch_up()
    });
    while got.len() < 40 {
        got.push(link.entry());
    }

    // `envelopes_of` checks that the mailbox's seqs are 1 to 40.
    assert_eq!(got, messages(1, &envelopes_of(s.db(), s.bob)));
    link.assert_quiet();
}
