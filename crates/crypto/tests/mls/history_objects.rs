//! The history's objects in `skald-proto`: `history-link`, `history-entry`, `history-plaintext` and
//! `history-body` of `docs/protocol/membership.md`, "Key chain" and "History entry", encoded as
//! "Signed objects" and `docs/protocol/README.md`, "Encoding", say. A decoded object keeps its
//! bytes and unknown fields, so it encodes back to the bytes it was decoded from.

use skald_proto::{
    Cbor, ChannelId, Content, DeviceId, HistoryBody, HistoryEntry, HistoryLink, HistoryPlaintext,
    MessageId, SignedBody,
};

use crate::cbor::{Value, bytes, text};
use crate::fields::{map, truncated_and_longer, with, without};
use crate::history::{
    LABEL, SENT, Sealed, body_cbor, content, content_fields, entry_cbor, link_cbor, plaintext_cbor,
    unknown,
};

const CHANNEL: ChannelId = ChannelId::from_bytes([0xc1; 16]);
const SENDER: DeviceId = DeviceId::from_bytes([0xd1; 16]);
const MSG_ID: MessageId = MessageId::from_bytes([0x4d; 16]);
const NONCE: [u8; 12] = [0x6e; 12];

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_decodes_into_its_fields_and_encodes_back_with_its_unknown_fields() {
    for unknown in [vec![], unknown()] {
        let encoded = link_cbor(
            Sealed {
                channel: CHANNEL,
                epoch: 7,
                nonce: &NONCE,
                ciphertext: &[0xee; 32],
            },
            5,
            unknown.clone(),
        );
        let link = HistoryLink::from_cbor(&encoded).expect("a link decodes");
        assert_eq!(
            (
                link.channel,
                link.epoch,
                link.prev,
                link.nonce,
                link.ciphertext.as_slice()
            ),
            (CHANNEL, 7, 5, NONCE, &[0xee; 32][..])
        );
        assert_eq!(link.unknown.is_empty(), unknown.is_empty());
        assert_eq!(link.to_cbor(), encoded, "byte for byte");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_decodes_into_its_fields_and_encodes_back_with_its_unknown_fields() {
    for unknown in [vec![], unknown()] {
        let encoded = entry_cbor(
            Sealed {
                channel: CHANNEL,
                epoch: 1,
                nonce: &NONCE,
                ciphertext: &[0xef; 100],
            },
            unknown.clone(),
        );
        let entry = HistoryEntry::from_cbor(&encoded).expect("an entry decodes");
        assert_eq!(
            (
                entry.channel,
                entry.epoch,
                entry.nonce,
                entry.ciphertext.as_slice()
            ),
            (CHANNEL, 1, NONCE, &[0xef; 100][..])
        );
        assert_eq!(entry.unknown.is_empty(), unknown.is_empty());
        assert_eq!(entry.to_cbor(), encoded, "byte for byte");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_plaintext_and_its_body_decode_into_their_fields_and_encode_back_with_their_unknown_fields() {
    assert_eq!(<HistoryBody as SignedBody>::LABEL, LABEL);
    // Content with a field of a later version too.
    let mut later_content = content_fields();
    later_content.push((30, text("later")));
    let later_content = map(&later_content);
    for (content, unknown) in [(content(), vec![]), (later_content, unknown())] {
        let body = body_cbor(SENDER, CHANNEL, MSG_ID, &content, unknown.clone());
        let decoded = HistoryBody::from_cbor(&body).expect("a body decodes");
        assert_eq!(
            (
                decoded.sender,
                decoded.channel,
                decoded.msg_id,
                decoded.timestamp
            ),
            (SENDER, CHANNEL, MSG_ID, SENT)
        );
        assert_eq!(decoded.content, Content::from_cbor(&content).unwrap());
        assert_eq!(decoded.content.to_cbor(), content);
        assert_eq!(decoded.unknown.is_empty(), unknown.is_empty());
        assert_eq!(decoded.to_cbor(), body, "the body, byte for byte");

        let encoded = plaintext_cbor(&body, &[0x5a; 64]);
        let plaintext = HistoryPlaintext::from_cbor(&encoded).expect("a plaintext decodes");
        assert_eq!(plaintext.body(), &decoded);
        assert_eq!(plaintext.body_bytes(), body.as_slice());
        assert_eq!(plaintext.signature(), &[0x5a; 64]);
        assert_eq!(plaintext.to_cbor(), encoded, "the plaintext, byte for byte");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_that_breaks_its_cddl_is_refused_without_a_panic() {
    let valid = vec![
        (1, bytes(CHANNEL.as_bytes())),
        (2, Value::Uint(3)),
        (3, Value::Uint(2)),
        (4, bytes(&NONCE)),
        (5, bytes(&[0xee; 32])),
    ];
    assert!(HistoryLink::from_cbor(&map(&valid)).is_ok());
    let mut refused = vec![
        ("epoch 1", with(&valid, 2, Value::Uint(1))),
        ("epoch 0", with(&valid, 2, Value::Uint(0))),
        ("prev 0", with(&valid, 3, Value::Uint(0))),
        ("a nonce of 11 bytes", with(&valid, 4, bytes(&[0x6e; 11]))),
        ("a nonce of 13 bytes", with(&valid, 4, bytes(&[0x6e; 13]))),
        ("a channel of 15 bytes", with(&valid, 1, bytes(&[0xc1; 15]))),
        ("a text ciphertext", with(&valid, 5, text("key"))),
        ("an array", Value::Array(vec![Value::Uint(1)]).encode()),
    ];
    for key in 1..=5 {
        refused.push(("a missing field", without(&valid, key)));
    }
    for (what, encoded) in &refused {
        assert!(HistoryLink::from_cbor(encoded).is_err(), "refused: {what}");
    }
    for encoded in truncated_and_longer(&map(&valid)) {
        assert!(
            HistoryLink::from_cbor(&encoded).is_err(),
            "refused: {encoded:02x?}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_that_breaks_its_cddl_is_refused_without_a_panic() {
    let valid = vec![
        (1, bytes(CHANNEL.as_bytes())),
        (2, Value::Uint(1)),
        (3, bytes(&NONCE)),
        (4, bytes(&[0xef; 100])),
    ];
    assert!(HistoryEntry::from_cbor(&map(&valid)).is_ok());
    let mut refused = vec![
        ("epoch 0", with(&valid, 2, Value::Uint(0))),
        ("a nonce of 11 bytes", with(&valid, 3, bytes(&[0x6e; 11]))),
        ("a channel of 17 bytes", with(&valid, 1, bytes(&[0xc1; 17]))),
        ("a text ciphertext", with(&valid, 4, text("entry"))),
    ];
    for key in 1..=4 {
        refused.push(("a missing field", without(&valid, key)));
    }
    for (what, encoded) in &refused {
        assert!(HistoryEntry::from_cbor(encoded).is_err(), "refused: {what}");
    }
    for encoded in truncated_and_longer(&map(&valid)) {
        assert!(
            HistoryEntry::from_cbor(&encoded).is_err(),
            "refused: {encoded:02x?}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_plaintext_or_body_that_breaks_its_cddl_is_refused_without_a_panic() {
    let valid = vec![
        (1, bytes(SENDER.as_bytes())),
        (2, bytes(CHANNEL.as_bytes())),
        (3, bytes(MSG_ID.as_bytes())),
        (4, Value::Uint(SENT)),
        (5, bytes(&content())),
    ];
    assert!(HistoryBody::from_cbor(&map(&valid)).is_ok());
    let mut refused = vec![
        ("a sender of 15 bytes", with(&valid, 1, bytes(&[0xd1; 15]))),
        ("a text timestamp", with(&valid, 4, text("now"))),
        (
            "content that is not content",
            with(&valid, 5, bytes(&[0xa0])),
        ),
        (
            "content not embedded",
            with(&valid, 5, Value::Map(vec![(4, Value::Uint(1))])),
        ),
        ("content that is not CBOR", with(&valid, 5, bytes(&[0xff]))),
    ];
    for key in 1..=5 {
        refused.push(("a missing field", without(&valid, key)));
    }
    for (what, encoded) in &refused {
        assert!(HistoryBody::from_cbor(encoded).is_err(), "refused: {what}");
    }
    let valid = map(&valid);
    for encoded in truncated_and_longer(&valid) {
        assert!(
            HistoryBody::from_cbor(&encoded).is_err(),
            "refused: {encoded:02x?}"
        );
    }

    let plaintext = plaintext_cbor(&valid, &[0x5a; 64]);
    assert!(HistoryPlaintext::from_cbor(&plaintext).is_ok());
    for (what, encoded) in [
        (
            "a signature of 63 bytes",
            plaintext_cbor(&valid, &[0x5a; 63]),
        ),
        (
            "a body that is not a body",
            plaintext_cbor(&content(), &[0x5a; 64]),
        ),
        (
            "three items",
            Value::Array(vec![bytes(&valid), bytes(&[0x5a; 64]), bytes(&[])]).encode(),
        ),
    ] {
        assert!(
            HistoryPlaintext::from_cbor(&encoded).is_err(),
            "refused: {what}"
        );
    }
    for encoded in truncated_and_longer(&plaintext) {
        assert!(
            HistoryPlaintext::from_cbor(&encoded).is_err(),
            "refused: {encoded:02x?}"
        );
    }
}
