//! Expiry by version ("Mailbox"; `docs/protocol/README.md`, "Protocol version"): a stored
//! envelope keeps its `v`, and the server sends it only on a connection whose version is `v` or
//! adjacent to it. On another, its entry expires when the server reaches it while sending, and an
//! envelope sent with `report` true gets `expired`. Against a server that accepts versions 1 and 2
//! as it stores the envelopes, then is restarted to accept 2 and 3.

use skald_proto::{Header, NonEmpty, ServerStatus};

use crate::expiring::{VERSIONS, restart};
use crate::link::{Entry, Link, assert_status, await_seqs};
use crate::scene::Scene;
use crate::wire::{assert_nowhere, envelope, now, version};

#[test]
fn a_message_of_version_1_expires_on_a_connection_of_version_3_and_is_sent_on_one_of_version_2() {
    let mut s = Scene::with(3_121_250_201, &[VERSIONS, "1-2"]);
    let carol = s.team().carol.laptop.id;
    // One message, of version 1, to bob and carol.
    let to_bob = s.fixture.to(s.bob);
    assert_eq!(to_bob.v, version(1), "an envelope of version 1");
    let to_carol = Header {
        recipients: Some(NonEmpty::new(carol)),
        ..to_bob.clone()
    };
    let for_bob = envelope(&to_bob);
    s.alice.store(&to_bob, &for_bob, true);
    let for_carol = envelope(&to_carol);
    s.alice.store(&to_carol, &for_carol, true);

    let server = restart(&mut s, &[VERSIONS, "2-3"], &[]);
    let t = s.team();

    let reached = now();
    let mut bob = Link::stating(&server, &t.flow, &t.bob, version(3));
    bob.sync(0);
    assert_eq!(bob.catch_up(), []);
    await_seqs(s.db(), s.bob, &[]);
    assert_nowhere(s.db(), &for_bob);

    let mut carol_link = Link::stating(&server, &t.flow, &t.carol, version(2));
    carol_link.sync(0);
    assert_eq!(carol_link.catch_up(), [Entry::Message(1, for_carol)]);
    let acked = now();
    carol_link.ack(1);
    await_seqs(s.db(), t.alice.laptop.id, &[1, 2]);

    let mut alice = Link::stating(&server, &t.flow, &t.alice, version(2));
    alice.sync(0);
    let entries = alice.catch_up();
    assert_eq!(entries.len(), 2, "expired, then delivered: {entries:?}");
    let expired = ServerStatus::Expired;
    assert_eq!(
        assert_status(&entries[0], expired, &to_bob, s.bob, reached),
        1
    );
    let delivered = ServerStatus::Delivered;
    assert_eq!(
        assert_status(&entries[1], delivered, &to_carol, carol, acked),
        2
    );
    alice.assert_quiet();
    bob.assert_quiet();
}
