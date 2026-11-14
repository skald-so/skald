//! Frames the server does not take as a `send` (`docs/protocol/delivery.md`, "Frames"): it ignores
//! a frame of a type it does not know, and the connection goes on; a frame that does not decode
//! closes its connection, and the server serves the next one.

use crate::fixture::Fixture;
use crate::wire::{
    Socket, array_of, assert_nowhere, byte_item, envelope, envelopes_of, send_frame, uint,
};

/// The CBOR text string `text`, up to 23 bytes.
fn short_text(text: &str) -> Vec<u8> {
    let len = u8::try_from(text.len()).expect("a short text");
    assert!(len < 24, "a short text");
    let mut item = vec![0x60 | len];
    item.extend_from_slice(text.as_bytes());
    item
}

#[test]
fn a_frame_of_an_unknown_type_is_ignored_and_the_connection_goes_on() {
    let fixture = Fixture::new(3_121_230_401);
    let t = &fixture.team;
    let mut alice = Socket::of(&t.flow, &t.alice);
    let before = fixture.to(t.bob.laptop.id);
    let first = envelope(&before);
    alice.store(&before, &first);

    let unknown = [
        array_of(&[uint(42)]),
        array_of(&[uint(99), short_text("later"), byte_item(&[1, 2, 3])]),
        array_of(&[uint(1_000), uint(1), uint(2), uint(3), uint(4)]),
    ];
    for frame in unknown {
        alice.frame(frame);
    }
    let after = fixture.to(t.bob.laptop.id);
    let second = envelope(&after);
    alice.store(&after, &second);

    assert_eq!(envelopes_of(fixture.db(), t.bob.laptop.id), [first, second]);
}

#[test]
fn a_malformed_frame_closes_its_connection_and_the_server_serves_the_next() {
    let fixture = Fixture::new(3_121_230_411);
    let t = &fixture.team;
    let probe = envelope(&fixture.to(t.bob.laptop.id));
    let full = send_frame(&probe);
    let mut non_shortest = vec![0x83, 0x18, 0x01];
    non_shortest.extend_from_slice(&full[2..]);
    let not_an_envelope = array_of(&[uint(1), byte_item(&[0x83, 0x01, 0x02, 0x03]), vec![0xf5]]);
    let cases = [
        ("not CBOR", vec![0xff]),
        ("a truncated send", full[..full.len() / 2].to_vec()),
        (
            "a send without report",
            array_of(&[uint(1), byte_item(&probe)]),
        ),
        (
            "a send with a report that is no bool",
            array_of(&[uint(1), byte_item(&probe), uint(1)]),
        ),
        ("a send whose envelope does not decode", not_an_envelope),
        (
            "a send whose type is not in the shortest form",
            non_shortest,
        ),
        ("bytes after the item", [full.as_slice(), &[0x00]].concat()),
    ];

    let mut stored = Vec::new();
    for (what, malformed) in cases {
        eprintln!("{what}");
        let mut alice = Socket::of(&t.flow, &t.alice);
        let header = fixture.to(t.bob.laptop.id);
        let sent = envelope(&header);
        alice.store(&header, &sent);
        stored.push(sent);

        alice.frame(malformed);

        alice.assert_closes();
        assert_eq!(
            envelopes_of(fixture.db(), t.bob.laptop.id),
            stored,
            "{what}"
        );
    }

    // Nothing of a malformed frame was stored, and the next connection is served.
    assert_nowhere(fixture.db(), &probe);
    let mut next = Socket::of(&t.flow, &t.alice);
    let header = fixture.to(t.carol.laptop.id);
    let sent = envelope(&header);
    next.store(&header, &sent);
    assert_eq!(envelopes_of(fixture.db(), t.carol.laptop.id), [sent]);
}
