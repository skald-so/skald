//! The spec's CBOR rules: `docs/protocol/README.md`, "Encoding". Malformed, non-deterministic and
//! truncated input is an error, never a panic.

mod common;

use common::{hex, vectors};
use skald_proto::{Cbor, DecodeError, EncodingProblem, MAX_DEPTH, Receipt};

/// The encoding of a 16-byte ID.
const ID: &str = "50019a6f1e2b3c7d4e8f5061a2b3c4d5e6";

fn receipt(text: &str) -> Result<Receipt, DecodeError> {
    Receipt::from_cbor(&hex(text))
}

fn encoding(at: usize, problem: EncodingProblem) -> Result<Receipt, DecodeError> {
    Err(DecodeError::Encoding { at, problem })
}

#[test]
fn a_valid_receipt_decodes() {
    assert!(receipt(&format!("a2 0103 0281{ID}")).is_ok());
}

#[test]
fn heads_must_be_in_their_shortest_form() {
    assert_eq!(
        receipt(&format!("a2 011803 0281{ID}")),
        encoding(2, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("a2 0103 029801{ID}")),
        encoding(4, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("a2 0103 0281 5810{}", &ID[2..])),
        encoding(5, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("b802 0103 0281{ID}")),
        encoding(0, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("a2 01190003 0281{ID}")),
        encoding(2, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("a2 011a00000003 0281{ID}")),
        encoding(2, EncodingProblem::NotShortest)
    );
    assert_eq!(
        receipt(&format!("a2 011b0000000000000003 0281{ID}")),
        encoding(2, EncodingProblem::NotShortest)
    );
    // The shortest forms at each boundary are accepted, as unknown fields.
    for value in [
        "17",
        "1818",
        "18ff",
        "190100",
        "19ffff",
        "1a00010000",
        "1b0000000100000000",
    ] {
        assert!(
            receipt(&format!("a3 0103 0281{ID} 04{value}")).is_ok(),
            "{value}"
        );
    }
    // `false` in two bytes.
    assert_eq!(
        receipt(&format!("a3 0103 0281{ID} 04f814")),
        encoding(23, EncodingProblem::NotShortest)
    );
}

#[test]
fn lengths_must_be_definite() {
    assert_eq!(
        receipt(&format!("bf 0103 0281{ID} ff")),
        encoding(0, EncodingProblem::IndefiniteLength)
    );
    assert_eq!(
        receipt(&format!("a2 0103 029f{ID}ff")),
        encoding(4, EncodingProblem::IndefiniteLength)
    );
    assert_eq!(
        receipt(&format!("a3 0103 0281{ID} 037f6178ff")),
        encoding(23, EncodingProblem::IndefiniteLength)
    );
    assert_eq!(
        receipt(&format!("a3 0103 0281{ID} 045f4100ff")),
        encoding(23, EncodingProblem::IndefiniteLength)
    );
}

#[test]
fn map_keys_must_be_sorted_and_unique() {
    assert_eq!(
        receipt(&format!("a2 0281{ID} 0103")),
        encoding(20, EncodingProblem::UnsortedKeys)
    );
    assert_eq!(
        receipt(&format!("a3 0103 0103 0281{ID}")),
        encoding(3, EncodingProblem::DuplicateKey)
    );
    // Bytewise order of the encodings: 24 (0x1818) before -1 (0x20), 23 (0x17) before 24.
    assert!(receipt(&format!("a5 0103 0281{ID} 1700 181800 2000")).is_ok());
    assert_eq!(
        receipt(&format!("a4 0103 0281{ID} 2000 181800")),
        encoding(24, EncodingProblem::UnsortedKeys)
    );
    // Nested maps are checked too.
    assert_eq!(
        receipt(&format!("a3 0103 0281{ID} 04a2 0200 0100")),
        encoding(26, EncodingProblem::UnsortedKeys)
    );
}

#[test]
fn tags_floats_and_other_simple_values_are_rejected() {
    let with = |value: &str| receipt(&format!("a3 0103 0281{ID} 04{value}"));
    assert_eq!(with("c1 00"), encoding(23, EncodingProblem::Tag));
    assert_eq!(with("c2 4101"), encoding(23, EncodingProblem::Tag));
    assert_eq!(with("f93c00"), encoding(23, EncodingProblem::Float));
    assert_eq!(with("fa3f800000"), encoding(23, EncodingProblem::Float));
    assert_eq!(
        with("fb3ff0000000000000"),
        encoding(23, EncodingProblem::Float)
    );
    assert_eq!(with("f7"), encoding(23, EncodingProblem::SimpleValue));
    assert_eq!(with("e0"), encoding(23, EncodingProblem::SimpleValue));
    assert_eq!(with("f820"), encoding(23, EncodingProblem::SimpleValue));
    for value in ["f4", "f5", "f6"] {
        assert!(with(value).is_ok(), "{value}");
    }
}

#[test]
fn text_must_be_utf8() {
    assert_eq!(
        receipt(&format!("a3 0103 0281{ID} 0362c328")),
        encoding(23, EncodingProblem::InvalidUtf8)
    );
    assert!(receipt(&format!("a3 0103 0281{ID} 0362c3a9")).is_ok());
}

#[test]
fn malformed_heads_are_rejected() {
    for reserved in ["1c", "1d", "1e", "3c", "5c", "7c", "9c", "bc", "dc", "fc"] {
        assert_eq!(
            receipt(&format!("a3 0103 0281{ID} 04{reserved}")),
            encoding(23, EncodingProblem::Malformed),
            "{reserved}"
        );
    }
    // A break code outside an indefinite-length item.
    assert_eq!(receipt("ff"), encoding(0, EncodingProblem::Malformed));
    // An integer with the indefinite-length marker.
    assert_eq!(receipt("1f"), encoding(0, EncodingProblem::Malformed));
}

#[test]
fn input_holds_exactly_one_item() {
    assert_eq!(
        receipt(&format!("a2 0103 0281{ID} 00")),
        Err(DecodeError::TrailingBytes(1))
    );
    assert_eq!(receipt(""), Err(DecodeError::Truncated));
}

#[test]
fn every_truncation_of_every_vector_is_rejected() {
    for vector in vectors() {
        for len in 0..vector.bytes.len() {
            assert_eq!(
                (vector.decode)(&vector.bytes[..len]),
                Err(DecodeError::Truncated),
                "{} cut to {len} bytes",
                vector.name
            );
        }
    }
}

#[test]
fn lengths_beyond_the_input_are_truncated() {
    for head in [
        "5bffffffffffffffff",
        "7bffffffffffffffff",
        "9bffffffffffffffff",
        "bbffffffffffffffff",
        "5a7fffffff",
        "9a7fffffff",
    ] {
        assert_eq!(receipt(head), Err(DecodeError::Truncated), "{head}");
        assert_eq!(
            receipt(&format!("a3 0103 0281{ID} 04{head}00")),
            Err(DecodeError::Truncated),
            "{head}"
        );
    }
}

#[test]
fn nesting_is_limited() {
    let nested = |depth: usize, inner: &str| format!("{}{inner}", "81".repeat(depth));
    // Under key 4 of a map, the deepest array sits at depth `MAX_DEPTH`.
    let deepest = format!("a3 0103 0281{ID} 04{}", nested(MAX_DEPTH - 1, "00"));
    assert!(receipt(&deepest).is_ok());
    let too_deep = format!("a3 0103 0281{ID} 04{}", nested(MAX_DEPTH, "00"));
    assert_eq!(
        receipt(&too_deep),
        encoding(23 + MAX_DEPTH - 1, EncodingProblem::TooDeep)
    );
    let maps = format!("a3 0103 0281{ID} 04{}00", "a104".repeat(MAX_DEPTH));
    assert_eq!(
        receipt(&maps),
        encoding(23 + 2 * (MAX_DEPTH - 1), EncodingProblem::TooDeep)
    );
    // Maps nested as keys count too: {{{...: 0}: 0}: 0}.
    let keys = format!(
        "{}{}",
        "a1".repeat(MAX_DEPTH + 1),
        "00".repeat(MAX_DEPTH + 2)
    );
    assert_eq!(
        receipt(&keys),
        encoding(MAX_DEPTH, EncodingProblem::TooDeep)
    );
    // Far deeper than any stack allows for recursion without a limit.
    let bytes = vec![0x81; 1_000_000];
    assert_eq!(
        Receipt::from_cbor(&bytes),
        encoding(MAX_DEPTH, EncodingProblem::TooDeep)
    );
}

/// Every one-byte change of every vector decodes to an error or to a value that encodes back to
/// the changed bytes: no panic, and the spec's encoding is the only one accepted.
#[test]
fn changed_vectors_are_rejected_or_round_trip() {
    for vector in vectors() {
        for at in 0..vector.bytes.len() {
            for mask in [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80, 0xff] {
                let mut bytes = vector.bytes.clone();
                bytes[at] ^= mask;
                if let Ok(encoded) = (vector.decode)(&bytes) {
                    assert_eq!(
                        encoded, bytes,
                        "{} with byte {at} ^ {mask:#04x}",
                        vector.name
                    );
                }
            }
        }
    }
}

#[test]
fn errors_explain_themselves() {
    use skald_proto::{AddressError, FieldProblem};

    let encoding = |problem| DecodeError::Encoding { at: 3, problem }.to_string();
    let cases = [
        (EncodingProblem::Malformed, "malformed CBOR"),
        (
            EncodingProblem::NotShortest,
            "a head not in its shortest form",
        ),
        (EncodingProblem::IndefiniteLength, "an indefinite length"),
        (
            EncodingProblem::UnsortedKeys,
            "a map key out of bytewise order",
        ),
        (EncodingProblem::DuplicateKey, "a repeated map key"),
        (EncodingProblem::Tag, "a tag"),
        (EncodingProblem::Float, "a floating-point value"),
        (
            EncodingProblem::SimpleValue,
            "a simple value other than false, true and null",
        ),
        (
            EncodingProblem::InvalidUtf8,
            "a text string that is not UTF-8",
        ),
        (EncodingProblem::TooDeep, "arrays and maps nested too deep"),
    ];
    for (problem, text) in cases {
        assert_eq!(encoding(problem), format!("at byte 3: {text}"));
    }

    let field = |problem| {
        DecodeError::Field {
            field: "header.v",
            problem,
        }
        .to_string()
    };
    let cases = [
        (FieldProblem::Missing, "missing"),
        (FieldProblem::Type("a map"), "not a map"),
        (
            FieldProblem::Length {
                expected: 16,
                actual: 15,
            },
            "15 bytes, not 16",
        ),
        (
            FieldProblem::Items {
                expected: 2,
                actual: 3,
            },
            "3 items, not 2",
        ),
        (FieldProblem::Empty, "empty; at least one item is needed"),
        (FieldProblem::Zero, "0; at least 1 is needed"),
        (FieldProblem::Value(7), "7 is not a defined value"),
        (FieldProblem::KeyType, "a map key that is not an integer"),
        (FieldProblem::NoBody, "no body"),
        (FieldProblem::TwoBodies, "more than one body"),
        (FieldProblem::NotAllowed, "not allowed here"),
        (
            FieldProblem::Address(AddressError::ExtraAt),
            "invalid agent address: more than one '@'",
        ),
        (
            FieldProblem::Embedded(Box::new(DecodeError::TrailingBytes(2))),
            "2 bytes follow the CBOR item",
        ),
    ];
    for (problem, text) in cases {
        assert_eq!(field(problem), format!("header.v: {text}"));
    }

    assert_eq!(
        DecodeError::Truncated.to_string(),
        "the CBOR item is truncated"
    );
}
