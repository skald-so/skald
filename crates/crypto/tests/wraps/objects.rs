//! A wrap's objects in `skald-proto`: `history-wrap`, `history-wrap-body` and `wrap-recipient` of
//! `docs/protocol/membership.md`, "Wraps", encoded as "Signed objects" and
//! `docs/protocol/README.md`, "Encoding", say. A decoded object keeps its bytes and unknown fields,
//! so it encodes back to the bytes it was decoded from.

use skald_proto::{
    AccountId, Cbor, ChannelId, DeviceId, HistoryWrap, HistoryWrapBody, SignedBody, WrapRecipient,
};

use crate::cbor::{Value, bytes, text};
use crate::fields::{map, truncated_and_longer, with, without};
use crate::fixtures::{ACCOUNT, RECOVERY};
use crate::fixtures::{
    Channel, Fields, LABEL, browser_reader, recovery_reader, unknown, wrap_cbor,
};

const CHANNEL: ChannelId = ChannelId::from_bytes([0xc1; 16]);
const BROWSER: DeviceId = DeviceId::from_bytes([0xb1; 16]);
const SIGNER: DeviceId = DeviceId::from_bytes([0xd1; 16]);
const RECOVERED: AccountId = AccountId::from_bytes([0xa1; 16]);

fn fields(recipient: WrapRecipient) -> Fields {
    Fields {
        channel: CHANNEL,
        epoch: 7,
        recipient,
        signer: SIGNER,
        enc: vec![0xe1; 32],
        ct: vec![0xc7; 32],
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_and_its_body_decode_into_their_fields_and_encode_back_with_their_unknown_fields() {
    assert_eq!(<HistoryWrapBody as SignedBody>::LABEL, LABEL);
    for recipient in [
        WrapRecipient::Browser(BROWSER),
        WrapRecipient::Recovery(RECOVERED),
    ] {
        for unknown in [vec![], unknown()] {
            let body = fields(recipient).encode(unknown.clone());
            let decoded = HistoryWrapBody::from_cbor(&body).expect("a wrap body decodes");
            assert_eq!(
                (
                    decoded.channel,
                    decoded.epoch,
                    decoded.recipient,
                    decoded.signer,
                    decoded.enc,
                    decoded.ct.as_slice(),
                ),
                (CHANNEL, 7, recipient, SIGNER, [0xe1; 32], &[0xc7; 32][..])
            );
            assert_eq!(decoded.unknown.is_empty(), unknown.is_empty());
            assert_eq!(decoded.to_cbor(), body, "the body, byte for byte");

            let encoded = wrap_cbor(&body, &[0x5a; 64]);
            let wrap = HistoryWrap::from_cbor(&encoded).expect("a wrap decodes");
            assert_eq!(wrap.body(), &decoded);
            assert_eq!(wrap.body_bytes(), body.as_slice());
            assert_eq!(wrap.signature(), &[0x5a; 64]);
            assert_eq!(wrap.to_cbor(), encoded, "the wrap, byte for byte");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_the_crate_makes_round_trips_byte_for_byte() {
    let channel = Channel::new(&[10, 11]);
    for reader in [browser_reader(50), recovery_reader(ACCOUNT, &RECOVERY)] {
        let wrap = channel.wrap(0, 1, &reader);
        let encoded = wrap.to_cbor();
        let decoded = HistoryWrap::from_cbor(&encoded).expect("the wrap decodes");
        assert_eq!(decoded, wrap);
        assert_eq!(decoded.to_cbor(), encoded);
        assert_eq!(wrap.body_bytes(), wrap.body().to_cbor().as_slice());
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_body_that_breaks_its_cddl_is_refused_without_a_panic() {
    let valid = fields(WrapRecipient::Browser(BROWSER)).entries();
    let id = |len: usize| bytes(&vec![0x11; len]);
    let recipient = |items: Vec<Value>| Value::Array(items);
    let mut bodies: Vec<(String, Vec<u8>)> = (1..=6)
        .map(|key| (format!("no field {key}"), without(&valid, key)))
        .collect();
    for (what, key, value) in [
        ("a 15-byte channel", 1, id(15)),
        ("a 17-byte channel", 1, id(17)),
        ("a text channel", 1, text("channel")),
        ("epoch 0", 2, Value::Uint(0)),
        ("a byte string epoch", 2, bytes(&[1])),
        (
            "a recipient of kind 0",
            3,
            recipient(vec![Value::Uint(0), id(16)]),
        ),
        (
            "a recipient of kind 3",
            3,
            recipient(vec![Value::Uint(3), id(16)]),
        ),
        (
            "a recipient without its ID",
            3,
            recipient(vec![Value::Uint(1)]),
        ),
        (
            "a recipient with a third item",
            3,
            recipient(vec![Value::Uint(1), id(16), id(16)]),
        ),
        (
            "a browser of 15 bytes",
            3,
            recipient(vec![Value::Uint(1), id(15)]),
        ),
        (
            "an account of 17 bytes",
            3,
            recipient(vec![Value::Uint(2), id(17)]),
        ),
        (
            "a text recipient ID",
            3,
            recipient(vec![Value::Uint(1), text("d")]),
        ),
        ("a recipient that is no array", 3, id(16)),
        ("an empty recipient", 3, recipient(vec![])),
        ("a 15-byte signer", 4, id(15)),
        ("a 31-byte enc", 5, bytes(&[0xe1; 31])),
        ("a 33-byte enc", 5, bytes(&[0xe1; 33])),
        ("an empty enc", 5, bytes(&[])),
        ("a text ct", 6, text("ct")),
    ] {
        bodies.push((what.to_owned(), with(&valid, key, value)));
    }
    let mut twice = valid.clone();
    twice.push((2, Value::Uint(8)));
    bodies.push(("epoch twice".to_owned(), map(&twice)));
    bodies.push(("an array".to_owned(), Value::Array(vec![]).encode()));
    for (what, body) in bodies {
        assert!(HistoryWrapBody::from_cbor(&body).is_err(), "{what}");
        assert!(
            HistoryWrap::from_cbor(&wrap_cbor(&body, &[0x5a; 64])).is_err(),
            "a wrap of a body with {what}"
        );
    }
    let body = map(&valid);
    for input in truncated_and_longer(&body) {
        assert!(HistoryWrapBody::from_cbor(&input).is_err(), "{input:02x?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_that_breaks_its_cddl_is_refused_without_a_panic() {
    let body = fields(WrapRecipient::Recovery(RECOVERED)).encode(vec![]);
    let malformed = [
        ("a 63-byte signature", wrap_cbor(&body, &[0x5a; 63])),
        ("a 65-byte signature", wrap_cbor(&body, &[0x5a; 65])),
        ("no signature", Value::Array(vec![bytes(&body)]).encode()),
        (
            "a third item",
            Value::Array(vec![bytes(&body), bytes(&[0x5a; 64]), bytes(&[])]).encode(),
        ),
        (
            "a body that is no byte string",
            Value::Array(vec![text("body"), bytes(&[0x5a; 64])]).encode(),
        ),
        ("a map", Value::Map(vec![(1, bytes(&body))]).encode()),
        ("no bytes", vec![]),
    ];
    for (what, input) in malformed {
        assert!(HistoryWrap::from_cbor(&input).is_err(), "{what}");
    }
    for input in truncated_and_longer(&wrap_cbor(&body, &[0x5a; 64])) {
        assert!(HistoryWrap::from_cbor(&input).is_err(), "{input:02x?}");
    }
}
