//! Acks of a channel's `mls` envelope, which has an entry in each recipient's mailbox
//! (`docs/protocol/delivery.md`, "Mailbox"): the server deletes the envelope once no entry refers
//! to it, also when its recipients ack it at once.

use std::sync::Barrier;
use std::thread;

use crate::common::TestDb;
use crate::link::{await_seqs, seqs};
use crate::scene::{Peer, Scene};
use crate::wire::query;

/// The number of envelopes the server holds.
fn envelopes(db: &TestDb) -> i64 {
    query(db, async |conn| {
        sqlx::query_scalar("SELECT count(*) FROM envelopes")
            .fetch_one(conn)
            .await
    })
}

#[test]
fn an_envelope_its_recipients_ack_at_once_is_deleted() {
    let (mut s, _) = Scene::formed(3_121_470_701);
    for round in 0..20 {
        let f = &s.fixture;
        let message = (s.alice.mls.encrypt(f.general, b"acked at once")).expect("a message");
        s.alice.stores(f, &message, &[]);
        let db = f.db();
        let acks: Vec<(&mut Peer, u64)> = [&mut s.bob, &mut s.carol]
            .into_iter()
            .map(|peer| {
                let last = *seqs(db, peer.id).last().expect("the message's entry");
                (peer, u64::try_from(last).expect("a seq from 1"))
            })
            .collect();
        let start = Barrier::new(acks.len());
        thread::scope(|scope| {
            for (peer, seq) in acks {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    peer.socket.ack(seq);
                });
            }
        });
        for peer in [&s.bob, &s.carol] {
            await_seqs(db, peer.id, &[]);
        }
        assert_eq!(
            envelopes(db),
            0,
            "round {round}: no envelope without an entry"
        );
    }
}
