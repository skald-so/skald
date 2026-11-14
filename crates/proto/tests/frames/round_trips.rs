//! Every frame encodes to the bytes the spec fixes and decodes back to the same frame.

use skald_proto::{Cbor, Frame, Push, SendKey};

use crate::common::{bob_device, hex, msg, stored, to_hex};
use crate::fixtures::{
    BOB, MSG_1, MSG_2, assert_round_trip, envelope_bstr, envelope_of, frames, hpke_envelope, raw,
    seq, stored_bytes,
};

#[test]
fn every_frame_type_round_trips_byte_for_byte() {
    for (name, frame, bytes) in frames() {
        assert_round_trip(name, &frame, &bytes);
    }
}

#[test]
fn a_stored_frame_reads_as_a_frame() {
    let bytes = stored_bytes();
    assert_round_trip("stored", &Frame::Stored(stored()), &to_hex(&bytes));
}

#[test]
fn a_send_without_a_report_round_trips() {
    assert_round_trip(
        "send, report false",
        &Frame::Send {
            envelope: raw(hpke_envelope()),
            report: false,
        },
        &format!("83 01 {} f4", envelope_bstr(&hpke_envelope())),
    );
}

#[test]
fn a_send_refused_under_hpke_names_its_recipient() {
    assert_round_trip(
        "send-refused with a recipient",
        &Frame::SendRefused {
            send_key: SendKey {
                msg_id: msg(2),
                recipient: Some(bob_device()),
            },
            problem: "unknown-recipient".to_owned(),
        },
        &format!(
            "83 03 82 50{MSG_2} 50{BOB} 71{}",
            to_hex(b"unknown-recipient")
        ),
    );
}

#[test]
fn a_send_refused_keeps_a_problem_proto_does_not_know() {
    // A client treats a problem it does not know as a refusal too.
    assert_round_trip(
        "send-refused with a later problem",
        &Frame::SendRefused {
            send_key: SendKey {
                msg_id: msg(1),
                recipient: None,
            },
            problem: "a-later-problem".to_owned(),
        },
        &format!("83 03 81 50{MSG_1} 6f{}", to_hex(b"a-later-problem")),
    );
}

#[test]
fn a_cursor_may_be_zero_and_counters_take_every_u64() {
    // `cursor` is any `uint`: 0 before the device has written an entry.
    assert_round_trip("sync from 0", &Frame::Sync { cursor: 0 }, "82 04 00");
    assert_round_trip(
        "sync from the largest cursor",
        &Frame::Sync { cursor: u64::MAX },
        "82 04 1bffffffffffffffff",
    );
    assert_round_trip(
        "ack of the largest seq",
        &Frame::Ack { seq: seq(u64::MAX) },
        "82 07 1bffffffffffffffff",
    );
    assert_round_trip(
        "ack with a two-byte seq",
        &Frame::Ack { seq: seq(1000) },
        "82 07 1903e8",
    );
}

#[test]
fn an_empty_batch_round_trips() {
    // The answer to a long poll that times out with nothing new.
    assert_round_trip("empty batch", &Frame::Batch(Vec::new()), "82 08 80");
}

#[test]
fn unknown_fields_of_a_delivery_status_round_trip() {
    // `delivered` with a later version's fields 5 and -1.
    let text = format!("83 06 09 a6 0150{MSG_1} 0250{BOB} 0301 041a69127c80 05 6178 20 f6");
    let bytes = hex(&text);
    let frame = Frame::from_cbor(&bytes).expect("a status with unknown fields");
    assert_eq!(to_hex(&frame.to_cbor()), to_hex(&bytes));
    let Frame::Push(Push::Status {
        seq: number,
        status,
    }) = frame
    else {
        panic!("not a status: {frame:?}");
    };
    assert_eq!(number, seq(9));
    assert_eq!(status.msg_id, msg(1));
    assert_eq!(status.unknown.keys().collect::<Vec<_>>(), [5, -1]);
    assert_eq!(status.unknown.get(5), Some(&hex("6178")[..]));
}

/// The "Envelope under `mls`" vector with a later version's header field 8: 82 bytes.
const ENVELOPE_WITH_A_LATER_FIELD: &str = "82 a6 0101 0250019a6f1e2b3c7d4e8f5061a2b3c4d5e6 \
    0350019a6f1e2b3c7d4e8f5061a2b3c4d5e7 0450019a6f201a007c008a00000000000001 0501 0800 \
    53000100020102030405060708090a0b0c0d0e0f";

#[test]
fn send_and_message_keep_the_envelope_bytes_they_came_in() {
    let envelope = hex(ENVELOPE_WITH_A_LATER_FIELD);
    let bstr = format!("5852 {ENVELOPE_WITH_A_LATER_FIELD}");
    for text in [format!("83 01 {bstr} f5"), format!("83 05 182a {bstr}")] {
        let bytes = hex(&text);
        let frame = Frame::from_cbor(&bytes).expect("a frame with a later envelope");
        assert_eq!(to_hex(&frame.to_cbor()), to_hex(&bytes), "{text}");
        let kept = match &frame {
            Frame::Send { envelope, report } => {
                assert!(report);
                envelope
            }
            Frame::Push(Push::Message {
                seq: number,
                envelope,
            }) => {
                assert_eq!(*number, seq(42));
                envelope
            }
            other => panic!("neither send nor message: {other:?}"),
        };
        assert_eq!(to_hex(kept.bytes()), to_hex(&envelope), "{text}");
        assert_eq!(kept.envelope(), &envelope_of(&envelope), "{text}");
        assert_eq!(
            kept.envelope().header.unknown.keys().collect::<Vec<_>>(),
            [8],
            "{text}"
        );
    }
}

#[test]
fn an_envelope_keeps_the_bytes_it_was_made_of() {
    let envelope = hex(ENVELOPE_WITH_A_LATER_FIELD);
    let kept = raw(envelope.clone());
    assert_eq!(to_hex(kept.bytes()), to_hex(&envelope));
    assert_eq!(kept.envelope(), &envelope_of(&envelope));
    assert_eq!(kept.envelope().header.msg_id, msg(1));
    assert_round_trip(
        "message with a later envelope",
        &Frame::Push(Push::Message {
            seq: seq(1),
            envelope: kept,
        }),
        &format!("83 05 01 5852 {ENVELOPE_WITH_A_LATER_FIELD}"),
    );
}
