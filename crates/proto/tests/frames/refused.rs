//! A frame of a type `proto` does not know decodes as unknown; malformed and truncated frames are
//! an error, never a panic.

use skald_proto::{Cbor, DecodeError, EncodingProblem, FieldProblem, Frame};

use crate::common::{assert_field_problem, hex, to_hex};
use crate::fixtures::{
    BOB, MSG_1, SPACE, delivered_hex, envelope_bstr, frame, frames, mls_envelope, send_hex,
    space_notice_hex, stored_bytes,
};

#[test]
fn a_frame_of_an_unknown_type_decodes_as_unknown() {
    for (text, frame_type) in [
        ("81 0a", 10),
        ("83 0a 6178 a1 01 02", 10),
        ("82 1903e8 40", 1000),
        ("82 1bffffffffffffffff 80", u64::MAX),
    ] {
        assert_eq!(frame(text), Ok(Frame::Unknown(frame_type)), "{text}");
    }
}

#[test]
fn a_frame_without_a_type_is_malformed_not_unknown() {
    for text in [
        "80",            // []
        "81 6473656e64", // ["send"]
        "81 20",         // [-1]
        "81 f5",         // [true]
        "81 4101",       // [h'01']
        "a1 01 02",      // {1: 2}
        "07",            // 7
        "f6",            // null
        "4401020304",    // a byte string
        "82 81 01 f5",   // [[1], true]
    ] {
        let result = frame(text);
        assert!(
            matches!(result, Err(DecodeError::Field { .. })),
            "{text}: {result:?}"
        );
    }
}

#[test]
fn a_frame_of_an_unknown_type_still_follows_the_encoding_rules() {
    assert_eq!(
        frame("82 0a 1805"),
        Err(DecodeError::Encoding {
            at: 2,
            problem: EncodingProblem::NotShortest
        })
    );
    assert_eq!(
        frame("82 0a a2 02 00 01 00"),
        Err(DecodeError::Encoding {
            at: 5,
            problem: EncodingProblem::UnsortedKeys
        })
    );
    assert_eq!(frame("82 0a"), Err(DecodeError::Truncated));
    assert_eq!(frame("81 0a 00"), Err(DecodeError::TrailingBytes(1)));
    // A known type in a non-shortest form is not an unknown type.
    assert_eq!(
        frame("82 1804 07"),
        Err(DecodeError::Encoding {
            at: 1,
            problem: EncodingProblem::NotShortest
        })
    );
}

#[test]
fn a_seq_of_zero_is_refused() {
    let envelope = envelope_bstr(&mls_envelope());
    for text in [
        format!("83 05 00 {envelope}"),
        format!("83 06 00 {}", delivered_hex()),
        "82 07 00".to_owned(),
        format!("82 08 81 83 05 00 {envelope}"),
        format!(
            "82 08 82 {} 83 06 00 {}",
            space_notice_hex(),
            delivered_hex()
        ),
    ] {
        assert_field_problem(frame(&text), &FieldProblem::Zero);
    }
}

#[test]
fn a_malformed_id_is_refused() {
    let short_space = &SPACE[..30];
    for (text, actual) in [
        (format!("82 09 4f{short_space}"), 15),
        (format!("82 09 51{SPACE}00"), 17),
        ("82 09 40".to_owned(), 0),
        (format!("83 03 81 4f{} 6178", &MSG_1[..30]), 15),
        (format!("83 03 82 50{MSG_1} 51{BOB}00 6178"), 17),
        (
            format!(
                "83 06 09 a4 014f{} 0250{BOB} 0301 041a69127c80",
                &MSG_1[..30]
            ),
            15,
        ),
        (
            format!(
                "83 06 09 a4 0150{MSG_1} 024f{} 0301 041a69127c80",
                &BOB[..30]
            ),
            15,
        ),
        (format!("82 08 81 82 09 4f{short_space}"), 15),
    ] {
        assert_field_problem(
            frame(&text),
            &FieldProblem::Length {
                expected: 16,
                actual,
            },
        );
    }
    // An ID in its text form is not an ID on the wire.
    let text_id = format!(
        "82 09 78 24 {}",
        to_hex(b"019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e6")
    );
    let result = frame(&text_id);
    assert!(
        matches!(
            result,
            Err(DecodeError::Field {
                problem: FieldProblem::Type(_),
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn an_envelope_that_does_not_decode_is_refused() {
    for text in [
        "83 01 4100 f5",
        "83 01 40 f5",
        "83 01 43 820102 f5",
        "83 05 01 4100",
        "83 05 01 40",
    ] {
        let result = frame(text);
        assert!(
            matches!(
                result,
                Err(DecodeError::Field {
                    problem: FieldProblem::Embedded(_),
                    ..
                })
            ),
            "{text}: {result:?}"
        );
    }
}

#[test]
fn malformed_frames_are_refused() {
    let envelope = envelope_bstr(&mls_envelope());
    // The envelope as an item of its own instead of a byte string that holds it.
    let inline = to_hex(&mls_envelope());
    let stored = to_hex(&stored_bytes());
    for text in [
        // send
        format!("82 01 {envelope}"),
        format!("83 01 {envelope} 01"),
        format!("83 01 {envelope} f6"),
        format!("83 01 {inline} f5"),
        format!("84 01 {envelope} f5 00"),
        "83 01 f5 f5".to_owned(),
        // send-refused
        format!("82 03 81 50{MSG_1}"),
        "83 03 80 6178".to_owned(),
        format!("83 03 83 50{MSG_1} 50{BOB} 50{BOB} 6178"),
        format!("83 03 81 50{MSG_1} 05"),
        format!("83 03 50{MSG_1} 6178"),
        format!("84 03 81 50{MSG_1} 6178 00"),
        // sync
        "81 04".to_owned(),
        "83 04 01 02".to_owned(),
        "82 04 20".to_owned(),
        "82 04 6131".to_owned(),
        // message
        "82 05 08".to_owned(),
        format!("83 05 08 {inline}"),
        format!("83 05 20 {envelope}"),
        format!("84 05 08 {envelope} 00"),
        // status
        "82 06 09".to_owned(),
        "83 06 09 a0".to_owned(),
        format!("83 06 09 a4 0150{MSG_1} 0250{BOB} 0303 041a69127c80"),
        format!("83 06 09 a3 0150{MSG_1} 0250{BOB} 0301"),
        format!("83 06 09 582d {}", delivered_hex()),
        // ack
        "81 07".to_owned(),
        "82 07 6131".to_owned(),
        "83 07 09 09".to_owned(),
        // batch
        "81 08".to_owned(),
        "82 08 a0".to_owned(),
        "83 08 80 80".to_owned(),
        format!("82 08 81 {}", send_hex()),
        format!("82 08 81 {stored}"),
        format!("82 08 81 83 03 81 50{MSG_1} 6178"),
        "82 08 81 82 04 07".to_owned(),
        "82 08 81 82 07 09".to_owned(),
        "82 08 81 82 08 80".to_owned(),
        "82 08 81 07".to_owned(),
        // space-notice
        "81 09".to_owned(),
        "82 09 f6".to_owned(),
        format!("83 09 50{SPACE} 00"),
        // stored
        format!("82 02 81 50{MSG_1}"),
    ] {
        let result = frame(&text);
        assert!(
            matches!(result, Err(DecodeError::Field { .. })),
            "{text}: {result:?}"
        );
    }
}

/// The bytes of every frame of [`frames`] and of `stored`.
fn every_frame() -> Vec<Vec<u8>> {
    let mut all: Vec<Vec<u8>> = frames()
        .into_iter()
        .map(|(_, _, text)| hex(&text))
        .collect();
    all.push(stored_bytes());
    all
}

#[test]
fn every_truncation_of_every_frame_is_truncated() {
    for bytes in every_frame() {
        for len in 0..bytes.len() {
            assert_eq!(
                Frame::from_cbor(&bytes[..len]),
                Err(DecodeError::Truncated),
                "{} cut to {len} bytes",
                to_hex(&bytes)
            );
        }
    }
}

#[test]
fn corrupted_frames_are_an_error_or_round_trip_never_a_panic() {
    for bytes in every_frame() {
        for at in 0..bytes.len() {
            let original = bytes[at];
            for byte in [
                0x00,
                0x17,
                0x18,
                0x1b,
                0x20,
                0x40,
                0x5b,
                0x7b,
                0x80,
                0x9b,
                0xa0,
                0xbb,
                0xf6,
                0xff,
                original ^ 0x01,
                original ^ 0x20,
                original.wrapping_add(1),
                original.wrapping_sub(1),
            ] {
                let mut corrupted = bytes.clone();
                corrupted[at] = byte;
                // An accepted frame of a known type re-encodes to the bytes it came in.
                if let Ok(decoded) = Frame::from_cbor(&corrupted)
                    && !matches!(decoded, Frame::Unknown(_))
                {
                    assert_eq!(
                        to_hex(&decoded.to_cbor()),
                        to_hex(&corrupted),
                        "{decoded:?}"
                    );
                }
            }
        }
    }
}
