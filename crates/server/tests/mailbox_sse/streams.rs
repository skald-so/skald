//! A stream's catch-up and pushes ("Mailbox" and "Receipts"): it sends the entries after the cursor
//! in `Last-Event-ID` in `seq` order, then the empty `batch`, at once on an empty mailbox, then
//! each new entry and `space-notice`, through any instance. Its cursor acks: the server deletes the
//! entries it covers and adds `delivered` for the sender.

use std::time::Instant;

use crate::fixture::Fixture;
use crate::https::{AT_ONCE, Https, SLACK, WAKE};
use crate::link::{Entry, QUIET, assert_delivered, await_seqs, bytes, messages, seqs};
use crate::sse::{Event, Sse, sent_by_alice};
use crate::wire::{Socket, assert_nowhere, envelope, now};

/// bob's laptop opens a stream on another instance from cursor 1 of its three entries: it gets the
/// second and third, then the empty `batch`; then, within a second each, an entry stored over
/// HTTPS and one over a WebSocket through the first instance, and the notice of a revision of
/// `acme`. eve's stream, outside `acme`, gets no notice.
#[test]
fn a_stream_sends_the_entries_after_its_cursor_then_the_empty_batch_then_what_comes() {
    let mut f = Fixture::new(3_141_420_001);
    let t = &f.team;
    let (bob_id, space) = (t.bob.laptop.id, t.space);
    let first = &t.flow.stand.server;
    let other = t.flow.stand.instance(&[], &[]);
    let sent = bytes(&sent_by_alice(&f, bob_id, 3));
    let mut bob = Sse::to(&other, &t.flow, &t.bob, &t.bob.laptop).open(Some(1));
    let mut eve = Sse::of(&t.flow, &f.eve).open(None);

    assert_eq!(bob.catch_up(), messages(2, &sent[1..]));
    assert_eq!(eve.catch_up(), []);

    let over_https = f.to(bob_id);
    let by_request = envelope(&over_https);
    Https::of(&t.flow, &t.alice).store(&over_https, &by_request);
    assert_eq!(bob.entry_within(WAKE), Entry::Message(4, by_request));
    let over_websocket = f.to(bob_id);
    let by_socket = envelope(&over_websocket);
    Socket::to(first, &t.flow, &t.alice, &t.alice.laptop).store(&over_websocket, &by_socket);
    assert_eq!(bob.entry_within(WAKE), Entry::Message(5, by_socket));

    f.revise(|_| {});
    assert_eq!(bob.data_within(WAKE), Event::Notice(space));
    eve.assert_quiet(QUIET);
}

/// A stream on an empty mailbox gets the empty `batch` at once.
#[test]
fn on_an_empty_mailbox_the_empty_batch_comes_at_once() {
    let f = Fixture::new(3_141_420_011);
    let bob = Sse::of(&f.team.flow, &f.team.bob);
    let proof = bob.https().proof();

    let opened = Instant::now();
    let mut stream = bob.open_on(None, &proof);

    let left = AT_ONCE.saturating_sub(opened.elapsed());
    assert_eq!(stream.data_within(left), Event::End);
}

/// alice's laptop sends bob's three envelopes with `report` true. bob's first stream, without
/// `Last-Event-ID`, gets all three and acks none; one opened again with `Last-Event-ID: 2` gets only
/// the third, and the first two are deleted, alice's stream getting `delivered` for each; one with
/// `Last-Event-ID: 3` gets nothing, and the mailbox is empty.
#[test]
fn a_stream_opened_again_with_last_event_id_gets_nothing_it_covers_and_that_is_deleted() {
    let f = Fixture::new(3_141_420_021);
    let t = &f.team;
    let bob_id = t.bob.laptop.id;
    let alice = Sse::of(&t.flow, &t.alice);
    let bob = Sse::of(&t.flow, &t.bob);
    let sends = sent_by_alice(&f, bob_id, 3);
    let sent = bytes(&sends);
    let mut to_alice = alice.open(None);
    assert_eq!(to_alice.catch_up(), []);

    assert_eq!(bob.open(None).catch_up(), messages(1, &sent));
    assert_eq!(seqs(f.db(), bob_id), [1, 2, 3], "acked without a cursor");

    let before = now();
    assert_eq!(bob.open(Some(2)).catch_up(), messages(3, &sent[2..]));
    await_seqs(f.db(), bob_id, &[3]);
    let statuses = [
        to_alice.entry_within(WAKE + SLACK),
        to_alice.entry_within(WAKE + SLACK),
    ];
    for ((header, envelope), seq) in sends[..2].iter().zip([1, 2]) {
        let status = statuses
            .iter()
            .find(
                |entry| matches!(entry, Entry::Status(_, status) if status.msg_id == header.msg_id),
            )
            .unwrap_or_else(|| panic!("no status of envelope {seq} in {statuses:?}"));
        assert_delivered(status, header, bob_id, before);
        assert_nowhere(f.db(), envelope);
    }

    assert_eq!(bob.open(Some(3)).catch_up(), []);
    await_seqs(f.db(), bob_id, &[]);
}
