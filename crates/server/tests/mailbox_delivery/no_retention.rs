//! Without the mailbox retention setting there is none, as a self-hosted server applies no plan
//! limits ("Mailbox"): a `message` without `ttl` stays until its device acks it, a `status` stays
//! until it is acked, and each idempotency key is kept for good. A `message` with `ttl` still
//! expires at its storing time plus the `ttl`.

use skald_proto::ServerStatus;

use crate::expiring::{EXPIRY, HOUR, YEAR, assert_retry_stores_nothing, move_back, send_bob_with};
use crate::link::{
    Entry, assert_delivered, assert_status, await_seqs, await_seqs_within, bytes, messages,
};
use crate::scene::Scene;
use crate::wire::now;

#[test]
fn without_the_setting_an_entry_without_ttl_stays_after_a_year() {
    let mut s = Scene::new(3_121_250_101);
    s.alice.sync(0);
    assert_eq!(s.alice.catch_up(), []);
    let (kept, first) = send_bob_with(&mut s, None, true);
    let (short, stored) = send_bob_with(&mut s, Some(HOUR), true);
    move_back(s.db(), YEAR);

    // The one with a `ttl` expires, so the server has looked for expired entries.
    let expired = s.alice.entry_within(EXPIRY);
    let at = stored.timestamp - YEAR + HOUR;
    assert_status(&expired, ServerStatus::Expired, &short.0, s.bob, at);
    await_seqs_within(s.db(), s.bob, &[1], EXPIRY);
    // The other one stays at `stored`.
    s.alice.assert_quiet();

    let mut link = s.bob_from(0, &[Entry::Message(1, kept.1.clone())]);
    let before = now();
    link.ack(1);
    assert_delivered(&s.alice.entry(), &kept.0, s.bob, before);
    await_seqs(s.db(), s.bob, &[]);

    // A year after, its key is kept.
    move_back(s.db(), YEAR);
    assert_retry_stores_nothing(&s, &kept, &first, &mut link);
}

#[test]
fn without_the_setting_a_status_stays_after_a_year() {
    let mut s = Scene::new(3_121_250_111);
    let alice = s.team().alice.laptop.id;
    let delivered = s.send_bob(1, true);
    let mut link = s.bob_from(0, &messages(1, &bytes(&delivered)));
    link.ack(1);
    await_seqs(s.db(), alice, &[1]);
    let (short, stored) = send_bob_with(&mut s, Some(HOUR), true);
    assert_eq!(link.entry(), Entry::Message(2, short.1.clone()));
    // Not acked.
    drop(link);

    move_back(s.db(), YEAR);
    await_seqs_within(s.db(), s.bob, &[], EXPIRY);
    await_seqs_within(s.db(), alice, &[1, 2], EXPIRY);

    s.alice.sync(0);
    let entries = s.alice.catch_up();
    let [Entry::Status(1, status), expired] = entries.as_slice() else {
        panic!("delivered, then expired, not {entries:?}");
    };
    // Its time moved back with the rest.
    let found = (status.status, status.msg_id, status.device);
    let msg_id = delivered[0].0.msg_id;
    assert_eq!(found, (ServerStatus::Delivered, msg_id, s.bob));
    let at = stored.timestamp - YEAR + HOUR;
    assert_eq!(
        assert_status(expired, ServerStatus::Expired, &short.0, s.bob, at),
        2
    );
    s.alice.assert_quiet();
}
