//! The idempotency key of an envelope (`docs/protocol/delivery.md`, "Send"): its sending device,
//! its `msg_id` and, under `hpke`, its recipient device. The server stores at most one envelope
//! per key and answers a retry as it answered the first send, whatever the retry's bytes.

use std::thread;
use std::time::Duration;

use crate::fixture::Fixture;
use crate::wire::{Answer, Socket, assert_nowhere, envelope, envelopes_of};

/// Past a second, so that a `stored` made anew would have a later timestamp.
const LATER: Duration = Duration::from_millis(1_100);

#[test]
fn a_retry_also_with_other_bytes_gets_the_first_stored_and_stores_nothing() {
    let fixture = Fixture::new(3_121_230_101);
    let t = &fixture.team;
    let header = fixture.to(t.bob.laptop.id);
    let sent = envelope(&header);
    let mut alice = Socket::of(&t.flow, &t.alice);
    let first = Answer::Stored(alice.store(&header, &sent));
    thread::sleep(LATER);

    alice.send(&sent);
    assert_eq!(alice.answer(), first, "a retry on the same connection");
    let mut next = Socket::of(&t.flow, &t.alice);
    next.send(&sent);
    assert_eq!(next.answer(), first, "a retry on another connection");

    let other_payload = envelope(&header);
    let mut with_ttl = header.clone();
    with_ttl.ttl = Some(3_600);
    let other_header = envelope(&with_ttl);
    for (what, other) in [
        ("another payload", &other_payload),
        ("another header", &other_header),
    ] {
        next.send(other);
        assert_eq!(next.answer(), first, "a retry with {what}");
        assert_nowhere(fixture.db(), other);
    }

    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [sent]);
}

#[test]
fn the_same_msg_id_from_another_device_is_another_envelope() {
    let fixture = Fixture::new(3_121_230_121);
    let t = &fixture.team;
    let from_alice = fixture.to(t.carol.laptop.id);
    let mut from_bob = fixture.to(t.carol.laptop.id);
    from_bob.msg_id = from_alice.msg_id;
    let (by_alice, by_bob) = (envelope(&from_alice), envelope(&from_bob));

    Socket::of(&t.flow, &t.alice).store(&from_alice, &by_alice);
    Socket::of(&t.flow, &t.bob).store(&from_bob, &by_bob);

    assert_eq!(
        envelopes_of(fixture.db(), t.carol.laptop.id),
        [by_alice, by_bob]
    );
}
