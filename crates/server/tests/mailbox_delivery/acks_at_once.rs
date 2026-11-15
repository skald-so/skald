//! Acks at once ("Receipts"): two recipient devices that ack at once the messages of two senders,
//! stored in their mailboxes in opposite orders, each finish, and each sender gets every
//! `delivered`.

use std::collections::HashSet;
use std::sync::Barrier;
use std::thread;

use skald_proto::{DeviceId, MessageId};

use crate::fixture::channel_members;
use crate::link::{Entry, await_seqs, store_to};
use crate::scene::Scene;

/// The rounds, and the messages each sender sends each recipient in a round.
const ROUNDS: usize = 5;
const EACH: usize = 10;

#[test]
fn two_devices_acking_at_once_the_messages_of_two_senders_each_finish() {
    let mut s = Scene::new(3_121_240_231);
    let t = s.team();
    let accounts = [
        t.alice.account,
        t.bob.account,
        t.carol.account,
        t.dave.account,
    ];
    let general = s.fixture.general;
    s.fixture
        .revise(|body| channel_members(body, general, &accounts));
    let (bob, carol) = (s.bob, s.team().carol.laptop.id);
    // alice's connection and dave's, which get the statuses.
    let mut senders = [s.link(&s.team().alice), s.link(&s.team().dave)];
    let mut readers = [s.link(&s.team().bob), s.link(&s.team().carol)];
    for link in senders.iter_mut().chain(&mut readers) {
        link.sync(0);
        assert_eq!(link.catch_up(), []);
    }

    for round in 1..=ROUNDS {
        // bob's mailbox gets alice's message first, carol's dave's.
        let mut sent: [HashSet<(MessageId, DeviceId)>; 2] = Default::default();
        for _ in 0..EACH {
            for (sender, recipient) in [(0, bob), (1, bob), (1, carol), (0, carol)] {
                let stored = store_to(&mut senders[sender], &s.fixture, recipient, 1, true);
                sent[sender].insert((stored[0].0.msg_id, recipient));
            }
        }
        let last = u64::try_from(round * 2 * EACH).unwrap();
        for reader in &mut readers {
            assert_eq!(reader.entries(2 * EACH).last().map(Entry::seq), Some(last));
        }

        let at_once = Barrier::new(2);
        thread::scope(|scope| {
            for reader in &mut readers {
                let at_once = &at_once;
                scope.spawn(move || {
                    at_once.wait();
                    reader.ack(last);
                });
            }
        });
        for device in [bob, carol] {
            await_seqs(s.db(), device, &[]);
        }

        for (sender, expected) in senders.iter_mut().zip(&sent) {
            let delivered: HashSet<(MessageId, DeviceId)> = sender
                .entries(2 * EACH)
                .iter()
                .map(|entry| match entry {
                    Entry::Status(_, status) => (status.msg_id, status.device),
                    other => panic!("a status, not {other:?}"),
                })
                .collect();
            assert_eq!(&delivered, expected, "round {round}");
        }
    }
    // A failed ack would have closed its connection.
    for reader in &mut readers {
        reader.assert_quiet();
    }
}
