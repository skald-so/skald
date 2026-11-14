//! Order (`docs/protocol/delivery.md`, "Order" and "Mailbox"): the server stores a connection's
//! sends in the order they arrive, a mailbox holds its entries in the order they were stored, and
//! each mailbox numbers its entries by `seq` from 1.

use std::collections::BTreeSet;
use std::sync::Barrier;
use std::thread;

use skald_proto::{DeviceId, Header};

use crate::fixture::Fixture;
use crate::wire::{Answer, Socket, envelope, envelopes_of, key_of};

/// `count` envelopes in `fixture`'s channel to `recipient`, with their headers.
fn envelopes_to(fixture: &Fixture, recipient: DeviceId, count: usize) -> Vec<(Header, Vec<u8>)> {
    (0..count)
        .map(|_| {
            let header = fixture.to(recipient);
            let sent = envelope(&header);
            (header, sent)
        })
        .collect()
}

/// Sends every one of `sends` on `socket` before reading any answer, then checks that each got
/// `stored`.
fn send_all(socket: &mut Socket, sends: &[(Header, Vec<u8>)]) {
    for (_, sent) in sends {
        socket.send(sent);
    }
    let mut stored = BTreeSet::new();
    for answer in socket.answers(sends.len()) {
        let Answer::Stored(answer) = answer else {
            panic!("stored, not {answer:?}");
        };
        stored.insert(format!("{:?}", answer.send_key));
    }
    let keys: BTreeSet<String> = sends
        .iter()
        .map(|(header, _)| format!("{:?}", key_of(header)))
        .collect();
    assert_eq!(stored, keys, "a stored for each send");
}

/// The envelopes of `sends`.
fn bytes(sends: &[(Header, Vec<u8>)]) -> Vec<Vec<u8>> {
    sends.iter().map(|(_, sent)| sent.clone()).collect()
}

#[test]
fn one_connections_sends_are_stored_in_the_order_they_arrive() {
    let fixture = Fixture::new(3_121_230_301);
    let t = &fixture.team;
    let to_bob = envelopes_to(&fixture, t.bob.laptop.id, 15);
    let to_carol = envelopes_to(&fixture, t.carol.laptop.id, 15);
    let interleaved: Vec<_> = to_bob
        .iter()
        .zip(&to_carol)
        .flat_map(|(bob, carol)| [bob.clone(), carol.clone()])
        .collect();

    send_all(&mut Socket::of(&t.flow, &t.alice), &interleaved);

    // Each mailbox numbers its own entries from 1.
    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), bytes(&to_bob));
    assert_eq!(
        envelopes_of(fixture.db(), t.carol.laptop.id),
        bytes(&to_carol)
    );
}

#[test]
fn a_mailbox_holds_its_entries_in_the_order_they_were_stored() {
    let fixture = Fixture::new(3_121_230_311);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);
    let mut carol = Socket::of(&t.flow, &t.carol);

    let mut stored = Vec::new();
    for round in 0..12 {
        // Each sender's envelope is stored before the other's is sent.
        let socket = if round % 3 == 0 {
            &mut carol
        } else {
            &mut alice
        };
        let header = fixture.to(t.bob.laptop.id);
        let sent = envelope(&header);
        socket.store(&header, &sent);
        stored.push(sent);
    }

    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), stored);
}

#[test]
fn two_devices_sending_at_once_keep_each_ones_order() {
    let fixture = Fixture::new(3_121_230_321);
    let t = &fixture.team;
    let bob = t.bob.laptop.id;
    let from_alice = envelopes_to(&fixture, bob, 25);
    let from_carol = envelopes_to(&fixture, bob, 25);
    let senders = [
        (Socket::of(&t.flow, &t.alice), &from_alice),
        (Socket::of(&t.flow, &t.carol), &from_carol),
    ];

    let start = Barrier::new(senders.len());
    thread::scope(|scope| {
        for (mut socket, sends) in senders {
            let start = &start;
            scope.spawn(move || {
                start.wait();
                send_all(&mut socket, sends);
            });
        }
    });

    // `envelopes_of` checks that the seqs are 1 to 50.
    let mailbox = envelopes_of(fixture.db(), bob);
    assert_eq!(mailbox.len(), 50, "each envelope once");
    for sends in [&from_alice, &from_carol] {
        let own = bytes(sends);
        let in_mailbox: Vec<Vec<u8>> = mailbox
            .iter()
            .filter(|entry| own.contains(entry))
            .cloned()
            .collect();
        assert_eq!(in_mailbox, own, "a sender's envelopes in its order");
    }
}
