//! Frames and their bytes, built by hand from the CDDL of `docs/protocol/delivery.md`, "Frames",
//! with the IDs and envelopes of `docs/protocol/test-vectors.md`.

use std::num::NonZeroU64;

use skald_proto::{Cbor, DecodeError, Envelope, Frame, Push, RawEnvelope, SendKey, ServerStatus};

use crate::common::{delivery_status, hex, msg, nonzero, space, to_hex, vectors};

/// `msg_id` of the first and the second message of the vectors.
pub const MSG_1: &str = "019a6f201a007c008a00000000000001";
pub const MSG_2: &str = "019a6f201a007c008a00000000000002";
/// Bob's device.
pub const BOB: &str = "188e6ae45b2d8247b1e1583b84bd49c7";
/// The space of the vectors.
pub const SPACE: &str = "019a6f1e2b3c7d4e8f5061a2b3c4d5e6";

/// The `delivery-status` of the `delivered` vector.
pub fn delivered_hex() -> String {
    format!("a4 0150{MSG_1} 0250{BOB} 0301 041a69127c80")
}

fn published(name: &str) -> Vec<u8> {
    vectors()
        .into_iter()
        .find(|vector| vector.name == name)
        .unwrap_or_else(|| panic!("no vector {name:?}"))
        .bytes
}

/// The bytes of the "Server status `stored`" vector.
pub fn stored_bytes() -> Vec<u8> {
    published("Server status `stored`")
}

/// The bytes of the "Envelope under `mls`" vector: 80 bytes.
pub fn mls_envelope() -> Vec<u8> {
    published("Envelope under `mls`")
}

/// The bytes of the "Envelope under `hpke`" vector: 139 bytes.
pub fn hpke_envelope() -> Vec<u8> {
    published("Envelope under `hpke`")
}

/// `bytes` as a byte string that holds an envelope: an 80- or 139-byte one.
pub fn envelope_bstr(bytes: &[u8]) -> String {
    let head = match bytes.len() {
        80 => "5850",
        139 => "588b",
        other => panic!("no head for an envelope of {other} bytes"),
    };
    format!("{head}{}", to_hex(bytes))
}

pub fn raw(bytes: Vec<u8>) -> RawEnvelope {
    RawEnvelope::from_bytes(bytes).expect("an envelope")
}

pub fn seq(value: u64) -> NonZeroU64 {
    nonzero(value)
}

pub fn send_frame() -> Frame {
    Frame::Send {
        envelope: raw(mls_envelope()),
        report: true,
    }
}

pub fn send_hex() -> String {
    format!("83 01 {} f5", envelope_bstr(&mls_envelope()))
}

pub fn message_push() -> Push {
    Push::Message {
        seq: seq(8),
        envelope: raw(hpke_envelope()),
    }
}

pub fn message_hex() -> String {
    format!("83 05 08 {}", envelope_bstr(&hpke_envelope()))
}

pub fn status_push() -> Push {
    Push::Status {
        seq: seq(9),
        status: delivery_status(ServerStatus::Delivered),
    }
}

pub fn status_hex() -> String {
    format!("83 06 09 {}", delivered_hex())
}

pub fn space_notice_push() -> Push {
    Push::SpaceNotice { space: space() }
}

pub fn space_notice_hex() -> String {
    format!("82 09 50{SPACE}")
}

/// One frame of each type and its bytes.
pub fn frames() -> Vec<(&'static str, Frame, String)> {
    vec![
        ("send", send_frame(), send_hex()),
        (
            "send-refused",
            Frame::SendRefused {
                send_key: SendKey {
                    msg_id: msg(1),
                    recipient: None,
                },
                problem: "not-a-member".to_owned(),
            },
            format!("83 03 8150{MSG_1} 6c{}", to_hex(b"not-a-member")),
        ),
        ("sync", Frame::Sync { cursor: 7 }, "82 04 07".to_owned()),
        ("message", Frame::Push(message_push()), message_hex()),
        ("status", Frame::Push(status_push()), status_hex()),
        ("ack", Frame::Ack { seq: seq(9) }, "82 07 09".to_owned()),
        (
            "batch",
            Frame::Batch(vec![message_push(), status_push(), space_notice_push()]),
            format!(
                "82 08 83 {} {} {}",
                message_hex(),
                status_hex(),
                space_notice_hex()
            ),
        ),
        (
            "space-notice",
            Frame::Push(space_notice_push()),
            space_notice_hex(),
        ),
    ]
}

/// `text` in hex, decoded as a frame.
pub fn frame(text: &str) -> Result<Frame, DecodeError> {
    Frame::from_cbor(&hex(text))
}

/// Asserts that `frame` encodes to exactly the bytes of `text` and decodes back from them.
pub fn assert_round_trip(name: &str, frame: &Frame, text: &str) {
    let bytes = hex(text);
    assert_eq!(to_hex(&frame.to_cbor()), to_hex(&bytes), "{name}: encoded");
    assert_eq!(
        Frame::from_cbor(&bytes).as_ref(),
        Ok(frame),
        "{name}: decoded"
    );
}

/// The envelope `bytes` hold, as `proto` decodes it.
pub fn envelope_of(bytes: &[u8]) -> Envelope {
    Envelope::from_cbor(bytes).expect("an envelope")
}
