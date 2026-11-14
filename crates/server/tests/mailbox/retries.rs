//! The idempotency key of an envelope (`docs/protocol/delivery.md`, "Send"): its sending device,
//! its `msg_id` and, under `hpke`, its recipient device. The server stores at most one envelope
//! per key and answers a retry as it answered the first send, whatever the retry's bytes.

use std::thread;
use std::time::Duration;

use skald_proto::{ChannelId, PROTOCOL_VERSION};

use crate::fixture::{Fixture, channel_members, exclude};
use crate::wire::{
    Answer, NOT_A_MEMBER, Socket, UNKNOWN_RECIPIENT, assert_nowhere, envelope, envelopes_of,
    version,
};

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

#[test]
fn a_retry_the_checks_would_refuse_gets_the_first_stored() {
    let mut fixture = Fixture::new(3_121_230_131);
    let t = &fixture.team;
    let header = fixture.to(t.carol.laptop.id);
    let sent = envelope(&header);
    let mut bob = Socket::of(&t.flow, &t.bob);
    let first = Answer::Stored(bob.store(&header, &sent));

    let mut other_version = header.clone();
    other_version.v = version(PROTOCOL_VERSION.get() + 1);
    let mut other_channel = header.clone();
    other_channel.channel = ChannelId::generate();
    for (what, retry) in [
        ("another v", envelope(&other_version)),
        ("a channel the sender is not in", envelope(&other_channel)),
    ] {
        bob.send(&retry);
        assert_eq!(bob.answer(), first, "a retry with {what}");
        assert_nowhere(fixture.db(), &retry);
    }

    let (account, laptop) = (fixture.team.bob.account, fixture.team.bob.laptop.id);
    fixture.revise(|body| exclude(body, account, &[laptop]));
    bob.send(&sent);
    assert_eq!(bob.answer(), first, "a retry after the sender is excluded");

    assert_eq!(
        envelopes_of(fixture.db(), fixture.team.carol.laptop.id),
        [sent]
    );
}

#[test]
fn a_retry_of_a_refused_send_gets_the_first_refusal() {
    let mut fixture = Fixture::new(3_121_230_141);
    let t = &fixture.team;
    // dave is in the space but not in the channel: his sends are refused, and so are sends to him.
    let from_dave = fixture.to(t.alice.laptop.id);
    let to_dave = fixture.to(t.dave.laptop.id);
    let (by_dave, for_dave) = (envelope(&from_dave), envelope(&to_dave));
    let mut dave = Socket::of(&t.flow, &t.dave);
    let mut alice = Socket::of(&t.flow, &t.alice);
    let first_from = dave.refusal(from_dave.msg_id, &by_dave);
    assert_eq!(first_from.1, NOT_A_MEMBER);
    let first_to = alice.refusal(to_dave.msg_id, &for_dave);
    assert_eq!(first_to.1, UNKNOWN_RECIPIENT);

    // Another `v` would be refused with another problem.
    let mut other_version = from_dave.clone();
    other_version.v = version(PROTOCOL_VERSION.get() + 1);
    let retry = envelope(&other_version);
    assert_eq!(dave.refusal(from_dave.msg_id, &retry), first_from);
    assert_nowhere(fixture.db(), &retry);

    // dave joins the channel.
    let general = fixture.general;
    let accounts = [
        t.alice.account,
        t.bob.account,
        t.carol.account,
        t.dave.account,
    ];
    fixture.revise(|body| channel_members(body, general, &accounts));
    let cases = [
        (&mut dave, &from_dave, &by_dave, &first_from),
        (&mut alice, &to_dave, &for_dave, &first_to),
    ];
    for (socket, header, sent, first) in cases {
        for retry in [sent.clone(), envelope(header)] {
            assert_eq!(&socket.refusal(header.msg_id, &retry), first);
            assert_nowhere(fixture.db(), &retry);
        }
    }

    // The revision admits a new send.
    let t = &fixture.team;
    let header = fixture.to(t.dave.laptop.id);
    let sent = envelope(&header);
    alice.store(&header, &sent);
    assert_eq!(envelopes_of(fixture.db(), t.dave.laptop.id), [sent]);
    assert_eq!(
        envelopes_of(fixture.db(), t.alice.laptop.id),
        Vec::<Vec<u8>>::new()
    );
}
