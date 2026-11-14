//! A channel's history: `docs/protocol/membership.md`, "Key chain" and "History entry". The
//! acceptance tests of the history are in `crates/crypto/tests/mls/`.

mod common;

use common::{a2a, alice, alice_device, assert_field_problem, channel, content, hex, msg, to_hex};
use skald_proto::{
    Body, Cbor, FieldProblem, HISTORY_LABEL, HistoryBody, HistoryEntry, HistoryLink,
    HistoryPlaintext, SignedBody, Unknown,
};

/// The hex of a 16-byte ID, a 12-byte nonce or a ciphertext of `n` bytes `byte`.
fn fill(byte: &str, n: usize) -> String {
    byte.repeat(n)
}

fn link(epoch: u64, prev: u64) -> HistoryLink {
    HistoryLink {
        channel: channel(),
        epoch,
        prev,
        nonce: [0x6e; 12],
        ciphertext: vec![0xee; 32],
        unknown: Unknown::default(),
    }
}

/// The encoded link of `channel()` from `epoch` to `prev`, both one-byte heads.
fn link_hex(epoch: &str, prev: &str) -> Vec<u8> {
    hex(&format!(
        "a5 0150{channel} 02{epoch} 03{prev} 044c{nonce} 055820{ciphertext}",
        channel = to_hex(channel().as_bytes()),
        nonce = fill("6e", 12),
        ciphertext = fill("ee", 32),
    ))
}

fn entry(epoch: u64) -> HistoryEntry {
    HistoryEntry {
        channel: channel(),
        epoch,
        nonce: [0x6f; 12],
        ciphertext: vec![0xef; 3],
        unknown: Unknown::default(),
    }
}

fn entry_hex(epoch: &str) -> Vec<u8> {
    hex(&format!(
        "a4 0150{channel} 02{epoch} 034c{nonce} 0443efefef",
        channel = to_hex(channel().as_bytes()),
        nonce = fill("6f", 12),
    ))
}

fn body() -> HistoryBody {
    HistoryBody {
        sender: alice_device(),
        channel: channel(),
        msg_id: msg(1),
        timestamp: 1_800_000_123,
        content: content(alice(), Body::A2a(a2a(msg(1), "hi"))),
        unknown: Unknown::default(),
    }
}

fn body_hex() -> Vec<u8> {
    let content = body().content.to_cbor();
    assert!(content.len() < 256, "a one-byte length");
    hex(&format!(
        "a5 0150{sender} 0250{channel} 0350{msg_id} 041a6b49d27b 0558{length:02x}{content}",
        sender = to_hex(alice_device().as_bytes()),
        channel = to_hex(channel().as_bytes()),
        msg_id = to_hex(msg(1).as_bytes()),
        length = content.len(),
        content = to_hex(&content),
    ))
}

#[test]
fn a_link_is_its_map_from_epoch_two_and_prev_one() {
    for (epoch, prev, encoded) in [
        (2, 1, link_hex("02", "01")),
        (24, 23, link_hex("1818", "17")),
    ] {
        assert_eq!(link(epoch, prev).to_cbor(), encoded);
        assert_eq!(HistoryLink::from_cbor(&encoded), Ok(link(epoch, prev)));
    }
    assert_field_problem(
        HistoryLink::from_cbor(&link_hex("01", "01")),
        &FieldProblem::Below { min: 2, actual: 1 },
    );
    assert_field_problem(
        HistoryLink::from_cbor(&link_hex("03", "00")),
        &FieldProblem::Below { min: 1, actual: 0 },
    );
}

#[test]
fn an_entry_is_its_map_from_epoch_one() {
    assert_eq!(entry(1).to_cbor(), entry_hex("01"));
    assert_eq!(HistoryEntry::from_cbor(&entry_hex("01")), Ok(entry(1)));
    assert_field_problem(
        HistoryEntry::from_cbor(&entry_hex("00")),
        &FieldProblem::Below { min: 1, actual: 0 },
    );
}

#[test]
fn a_plaintext_is_its_body_signed_over_skald_history() {
    assert_eq!(HISTORY_LABEL, "skald history");
    assert_eq!(<HistoryBody as SignedBody>::LABEL, HISTORY_LABEL);
    assert_eq!(body().to_cbor(), body_hex());
    assert_eq!(HistoryBody::from_cbor(&body_hex()), Ok(body()));
    let plaintext = HistoryPlaintext::from_parts(body_hex(), [0x5a; 64]).unwrap();
    assert_eq!(plaintext.body(), &body());
    let length = body_hex().len();
    let head = if length < 256 {
        format!("58{length:02x}")
    } else {
        format!("59{length:04x}")
    };
    assert_eq!(
        plaintext.to_cbor(),
        hex(&format!(
            "82 {head}{body} 5840{signature}",
            body = to_hex(&body_hex()),
            signature = fill("5a", 64),
        ))
    );
}

#[test]
fn the_associated_data_is_the_label_channel_and_epochs() {
    let channel_hex = to_hex(channel().as_bytes());
    assert_eq!(
        HistoryLink::associated_data(channel(), 24, 2),
        hex(&format!(
            "84 72{} 50{channel_hex} 1818 02",
            to_hex(b"skald history link")
        ))
    );
    assert_eq!(
        HistoryEntry::associated_data(channel(), 1),
        hex(&format!(
            "83 73{} 50{channel_hex} 01",
            to_hex(b"skald history entry")
        ))
    );
}
