//! The `auth` frame of `docs/protocol/delivery.md`, "Connecting": it round-trips byte for byte,
//! with a challenge of any length, and a malformed or truncated one is an error.

mod common;

use common::{alice_device, assert_field_problem, hex, to_hex};
use skald_proto::{Cbor, DecodeError, FieldProblem, Frame};

/// Alice's device ID, in hex.
const ALICE: &str = "19ac5f784938823586a6c5809a18f6dd";

/// The hex of a 64-byte signature of `0x5a`s, as a byte string.
fn signature() -> String {
    format!("5840{}", "5a".repeat(64))
}

fn frame(text: &str) -> Result<Frame, DecodeError> {
    Frame::from_cbor(&hex(text))
}

#[test]
fn auth_round_trips_with_a_challenge_of_any_length() {
    for (challenge, head) in [
        (vec![], "40"),
        (vec![7; 16], "50"),
        (vec![9; 300], "59012c"),
    ] {
        let auth = Frame::Auth {
            challenge: challenge.clone(),
            device: alice_device(),
            signature: [0x5a; 64],
        };
        let text = format!(
            "84 0a {head}{} 50{ALICE} {}",
            to_hex(&challenge),
            signature()
        );

        assert_eq!(to_hex(&auth.to_cbor()), to_hex(&hex(&text)));
        assert_eq!(frame(&text), Ok(auth));
    }
}

#[test]
fn auth_with_a_malformed_field_is_refused() {
    let signature = signature();
    let short_signature = format!("583f{}", "5a".repeat(63));
    for (text, problem) in [
        (
            format!("84 0a 4101 50{ALICE} {short_signature}"),
            FieldProblem::Length {
                expected: 64,
                actual: 63,
            },
        ),
        (
            format!("84 0a 4101 4f{} {signature}", &ALICE[..30]),
            FieldProblem::Length {
                expected: 16,
                actual: 15,
            },
        ),
    ] {
        assert_field_problem(frame(&text), &problem);
    }
    for text in [
        format!("83 0a 4101 50{ALICE}"),
        format!("85 0a 4101 50{ALICE} {signature} 00"),
        format!("84 0a 6101 50{ALICE} {signature}"),
        format!("84 0a 4101 50{ALICE} 00"),
    ] {
        let result = frame(&text);
        assert!(
            matches!(result, Err(DecodeError::Field { .. })),
            "{text}: {result:?}"
        );
    }
}

#[test]
fn truncated_auth_is_refused() {
    let bytes = hex(&format!("84 0a 4101 50{ALICE} {}", signature()));
    for len in 0..bytes.len() {
        assert_eq!(
            Frame::from_cbor(&bytes[..len]),
            Err(DecodeError::Truncated),
            "{len} bytes"
        );
    }
}
