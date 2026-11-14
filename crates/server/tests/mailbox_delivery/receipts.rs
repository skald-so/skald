//! `delivered` ("Receipts"): when an ack covers a `message` whose envelope was sent with `report`
//! true, the server adds `delivered`, with the recipient device and the time, to the sending
//! device's mailbox; for `report` false it adds nothing.

use skald_proto::Header;

use crate::link::{Entry, assert_delivered, await_seqs, bytes, messages, seqs};
use crate::scene::Scene;
use crate::wire::{envelope, now};

#[test]
fn the_sender_gets_delivered_for_report_true_only() {
    let mut s = Scene::new(3_121_240_201);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let mut sent = s.send_bob(1, true);
    sent.extend(s.send_bob(1, false));
    sent.extend(s.send_bob(1, true));
    let [reported, unreported, last] = [0, 1, 2].map(|i| sent[i].0.clone());
    s.alice.assert_quiet();

    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));
    let before = now();
    link.ack(3);

    let mut statuses = s.alice.entries(2);
    statuses.sort_by_key(Entry::seq);
    let of = |header: &Header| -> Vec<&Entry> {
        let msg_id = header.msg_id;
        let reports = |entry: &&Entry| matches!(entry, Entry::Status(_, s) if s.msg_id == msg_id);
        statuses.iter().filter(reports).collect()
    };
    assert_eq!(of(&unreported), Vec::<&Entry>::new(), "for report false");
    for header in [&reported, &last] {
        let found = of(header);
        assert_eq!(found.len(), 1, "one delivered: {statuses:?}");
        assert_delivered(found[0], header, s.bob, before);
    }
    let added: Vec<u64> = statuses.iter().map(Entry::seq).collect();
    assert_eq!(added, [1, 2]);
    s.alice.assert_quiet();
    assert_eq!(seqs(s.db(), s.team().alice.laptop.id), [1, 2]);
}

#[test]
fn delivered_names_each_recipient_device_also_for_an_ack_by_sync() {
    let mut s = Scene::new(3_121_240_211);
    let carol = s.team().carol.laptop.id;
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let to_bob = s.fixture.to(s.bob);
    let mut to_carol = s.fixture.to(carol);
    to_carol.msg_id = to_bob.msg_id;
    let (for_bob, for_carol) = (envelope(&to_bob), envelope(&to_carol));
    s.alice.store(&to_bob, &for_bob, true);
    s.alice.store(&to_carol, &for_carol, true);

    // carol's first connection drops before its ack: nothing is delivered yet.
    drop(s.synced(&s.team().carol, 0, &[Entry::Message(1, for_carol)]));

    let mut link = s.bob_from(0, &[Entry::Message(1, for_bob)]);
    let before = now();
    link.ack(1);
    assert_eq!(
        assert_delivered(&s.alice.entry(), &to_bob, s.bob, before),
        1
    );
    s.alice.assert_quiet();

    let before = now();
    s.synced(&s.team().carol, 1, &[]);
    let delivered = s.alice.entry();
    assert_eq!(assert_delivered(&delivered, &to_carol, carol, before), 2);
    s.alice.assert_quiet();
}

#[test]
fn a_sender_that_connects_after_delivered_was_added_gets_it_once() {
    let mut s = Scene::new(3_121_240_221);
    let alice = s.team().alice.laptop.id;
    let sent = s.send_bob(1, true);
    s.reconnect_alice();

    let mut link = s.bob_from(0, &messages(1, &bytes(&sent)));
    let before = now();
    link.ack(1);
    await_seqs(s.db(), alice, &[1]);
    // Acking it again, by `ack` and by the next connection's `sync`, adds nothing.
    link.ack(1);
    drop(link);
    let mut next = s.bob_from(1, &[]);

    s.alice.sync(0);
    let entries = s.alice.catch_up();
    assert_eq!(entries.len(), 1, "one delivered: {entries:?}");
    assert_eq!(assert_delivered(&entries[0], &sent[0].0, s.bob, before), 1);
    s.alice.assert_quiet();

    // Acking a `status` deletes it and adds no entry anywhere.
    s.alice.ack(1);
    await_seqs(s.db(), alice, &[]);
    next.assert_quiet();
    assert_eq!(seqs(s.db(), s.bob), Vec::<i64>::new());
}
