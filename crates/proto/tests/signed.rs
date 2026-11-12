//! What a signature covers (`docs/protocol/README.md`, "Conventions"), and a certificate made of
//! its parts, as an issuer and its device sign it (`docs/protocol/membership.md`, "Device
//! certificate").

mod common;

use common::hex;
use skald_proto::{Cbor, Cert, DecodeError, FieldProblem, signed_data};

#[test]
fn signed_data_is_the_array_of_the_label_and_byte_strings() {
    assert_eq!(
        signed_data("skald cert", &[b"ab", &[0; 24]]),
        hex(&format!(
            "83 6a{} 42 6162 5818{}",
            "736b616c642063657274",
            "00".repeat(24)
        ))
    );
    assert_eq!(signed_data("", &[]), hex("81 60"));
}

#[test]
fn a_certificate_of_its_parts_is_the_decoded_one() {
    // A body with the required fields only: 1 to 5, 7 and 9.
    let body = hex(&format!(
        "a7 0150{account} 0250{device} 0301 045820{key} 055820{key} 071a6911a780 0964{name}",
        account = "019a6f1e2b3c7d4e8f5061a2b3c4d5e6",
        device = "11".repeat(16),
        key = "22".repeat(32),
        name = "4c617074",
    ));
    let cert = Cert::from_parts(body.clone(), [1; 64], [2; 64]).unwrap();
    assert_eq!(cert.body_bytes(), body.as_slice());
    assert_eq!((cert.signature(), cert.proof()), (&[1; 64], &[2; 64]));
    assert_eq!(Cert::from_cbor(&cert.to_cbor()), Ok(cert));
}

#[test]
fn a_certificate_of_a_body_that_breaks_the_cddl_is_an_error() {
    let error = Cert::from_parts(hex("a0"), [1; 64], [2; 64]).unwrap_err();
    assert_eq!(
        error,
        DecodeError::Field {
            field: "cert.body",
            problem: FieldProblem::Embedded(Box::new(DecodeError::Field {
                field: "cert-body.account",
                problem: FieldProblem::Missing,
            })),
        }
    );
    assert!(Cert::from_parts(Vec::new(), [1; 64], [2; 64]).is_err());
}
