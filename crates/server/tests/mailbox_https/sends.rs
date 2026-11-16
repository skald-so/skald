//! Sends over HTTPS ("Send" and "Order"): each gets `stored` or `send-refused` as on a WebSocket,
//! stores its envelope once under its idempotency key, whichever transport sent it first, and the
//! requests state the protocol version.

use std::thread;
use std::time::Duration;

use crate::common::device_flow::json;
use crate::fixture::Fixture;
use crate::https::{ACK, Https, POLL, SEND, ack_frame, assert_problem, poll_path, sync_frame};
use crate::link::{bytes, messages};
use crate::wire::{
    Answer, NOT_A_MEMBER, Socket, UNKNOWN_RECIPIENT, VERSION_MISMATCH, assert_nowhere, envelope,
    envelopes_of, key_of, send_frame, version,
};

/// Past a second, so that a `stored` made anew would have a later timestamp.
const LATER: Duration = Duration::from_millis(1_100);
/// The server setting of the protocol versions it accepts.
const VERSIONS: &str = "--protocol-versions";

/// alice's laptop sends an envelope to bob's over HTTPS: it is stored once, and a retry, also
/// with other bytes, gets the first `stored`; an envelope sent over a WebSocket and retried over
/// HTTPS gets its first `stored` too.
#[test]
fn a_send_is_stored_once_and_a_retry_gets_the_first_answer() {
    let f = Fixture::new(3_141_410_001);
    let t = &f.team;
    let alice = Https::of(&t.flow, &t.alice);
    let bob = t.bob.laptop.id;
    let header = f.to(bob);
    let sent = envelope(&header);
    let first = Answer::Stored(alice.store(&header, &sent));
    thread::sleep(LATER);

    assert_eq!(alice.send(&sent, true), first, "a retry");
    let mut with_ttl = header.clone();
    with_ttl.ttl = Some(3_600);
    let other = envelope(&with_ttl);
    assert_eq!(alice.send(&other, true), first, "a retry with other bytes");
    assert_nowhere(f.db(), &other);

    let over_websocket = f.to(bob);
    let by_socket = envelope(&over_websocket);
    let stored = Socket::of(&t.flow, &t.alice).store(&over_websocket, &by_socket);
    thread::sleep(LATER);
    assert_eq!(
        alice.send(&by_socket, true),
        Answer::Stored(stored),
        "a retry over HTTPS of a send over a WebSocket"
    );

    assert_eq!(envelopes_of(f.db(), bob), [sent, by_socket]);
}

/// A send the server refuses gets `send-refused` with its problem, and nothing of it is stored:
/// from eve's laptop, outside `acme`, `not-a-member`; from alice's to dave's, who is in no channel,
/// `unknown-recipient`.
#[test]
fn a_refused_send_gets_its_problem_and_stores_nothing() {
    let f = Fixture::new(3_141_410_011);
    let t = &f.team;
    let eve = Https::of(&t.flow, &f.eve);
    let alice = Https::of(&t.flow, &t.alice);

    for (who, sender, header, problem) in [
        ("eve to alice", &eve, f.to(t.alice.laptop.id), NOT_A_MEMBER),
        (
            "alice to dave",
            &alice,
            f.to(t.dave.laptop.id),
            UNKNOWN_RECIPIENT,
        ),
    ] {
        let sent = envelope(&header);
        let refused = Answer::Refused(key_of(&header), problem.to_owned());
        assert_eq!(sender.send(&sent, true), refused, "{who}");
        assert_nowhere(f.db(), &sent);
    }

    for device in f.laptops() {
        assert_eq!(envelopes_of(f.db(), device), Vec::<Vec<u8>>::new());
    }
}

/// On a server that accepts versions 1 and 2, a send of an envelope of version 1 in a request
/// that states 2 is refused with `version-mismatch`; a send, an ack and a long-poll that state no
/// version are refused with the problem `protocol-version-invalid`. Neither stores anything; a send
/// of another envelope stating 1 is stored, as a retry of the refused one gets its refusal.
#[test]
fn requests_follow_the_protocol_version() {
    let f = Fixture::with(3_141_410_021, &[VERSIONS, "1-2"]);
    let t = &f.team;
    let bob = t.bob.laptop.id;
    let header = f.to(bob);
    assert_eq!(header.v, version(1), "an envelope of version 1");
    let sent = envelope(&header);

    let stating_2 = Https::of(&t.flow, &t.alice).stating(version(2));
    let mismatch = Answer::Refused(key_of(&header), VERSION_MISMATCH.to_owned());
    assert_eq!(stating_2.send(&sent, true), mismatch);
    assert_nowhere(f.db(), &sent);

    let alice = Https::of(&t.flow, &t.alice);
    let unstated = [
        (SEND, SEND.to_owned(), send_frame(&sent, true)),
        (POLL, poll_path(0), sync_frame(0)),
        (ACK, ACK.to_owned(), ack_frame(1)),
    ];
    for (operation, path, body) in unstated {
        let response = alice
            .request(operation, &path, body, Some(alice.proof()), false)
            .call();
        assert_problem(&response, 400, None);
        assert_eq!(
            json(&response)["type"],
            "https://skald.so/problems/protocol-version-invalid",
            "{operation}"
        );
    }
    assert_nowhere(f.db(), &sent);

    let next = f.to(bob);
    let stored = envelope(&next);
    alice.store(&next, &stored);
    assert_eq!(envelopes_of(f.db(), bob), [stored]);
}

/// Sends over HTTPS, each after the previous one is answered, are stored and delivered in that
/// order.
#[test]
fn sends_are_stored_in_the_order_they_are_answered() {
    let f = Fixture::new(3_141_410_031);
    let t = &f.team;
    let alice = Https::of(&t.flow, &t.alice);
    let bob = t.bob.laptop.id;

    let sent = bytes(&crate::https::stored_through(&alice, &f, bob, 4));

    assert_eq!(envelopes_of(f.db(), bob), sent);
    let entries = Https::of(&t.flow, &t.bob).poll_now(0).entries;
    assert_eq!(entries, messages(1, &sent));
}
