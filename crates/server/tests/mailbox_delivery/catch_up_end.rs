//! The end of catch-up ("Mailbox"): the empty `batch` comes once catch-up has sent every entry the
//! mailbox held at the first `sync`, so an entry added during catch-up comes after it.

use skald_proto::{Cbor as _, Envelope, HpkePayload};

use crate::common::devices::random;
use crate::link::{Entry, store_to};
use crate::scene::Scene;

/// How many entries the mailbox holds at the first `sync`, and the size of each one's ciphertext,
/// below the 64 KiB the tests' frames take: 24 MB together, far more than the connection's
/// buffers, so the server is still sending them when the test adds an entry.
const HELD: u64 = 400;
const CT: usize = 60_000;

#[test]
fn an_entry_added_during_catch_up_comes_after_its_empty_batch() {
    let mut s = Scene::new(3_121_240_051);
    let mut carol = s.link(&s.team().carol);
    let held: Vec<Vec<u8>> = (0..HELD)
        .map(|_| {
            let header = s.fixture.to(s.bob);
            let payload = HpkePayload {
                enc: random(),
                ct: vec![0; CT],
            };
            let sent = Envelope {
                header: header.clone(),
                payload: payload.to_cbor(),
            }
            .to_cbor();
            s.alice.store(&header, &sent, true);
            sent
        })
        .collect();
    let mut link = s.link(&s.team().bob);

    link.sync(0);
    // Catch-up has started: the server knows what the mailbox holds.
    let first = link.entry();
    let added = store_to(&mut carol, &s.fixture, s.bob, 1, true);
    let mut caught_up = vec![first];
    caught_up.extend(link.catch_up());

    let seqs: Vec<u64> = caught_up.iter().map(Entry::seq).collect();
    assert_eq!(
        seqs,
        (1..=HELD).collect::<Vec<_>>(),
        "the entries before the empty batch"
    );
    for (entry, sent) in caught_up.iter().zip(&held) {
        // Not compared with `assert_eq!`, which would print the envelopes.
        assert!(
            matches!(entry, Entry::Message(_, got) if got == sent),
            "an envelope as sent"
        );
    }
    assert_eq!(link.entry(), Entry::Message(HELD + 1, added[0].1.clone()));
    link.assert_quiet();
}
