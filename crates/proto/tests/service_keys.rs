//! A `service` device's key secret decodes into the fields it encodes, keeps the fields of later
//! versions, and its debug form shows no private key: `docs/protocol/membership.md`, "Service
//! device".

mod common;

use common::{account, assert_field_problem, space};
use skald_proto::{Cbor, FieldProblem, ServiceKey, Unknown};

fn key() -> ServiceKey {
    ServiceKey {
        signing: [0x11; 32],
        hpke: [0x22; 32],
        account: account(),
        master: [0x33; 32],
        space: space(),
        anchor: [0x44; 32],
        server: "https://skald.example.com".to_owned(),
        unknown: Unknown::default(),
    }
}

#[test]
fn a_key_secret_decodes_into_the_fields_it_encodes() {
    let key = key();
    assert_eq!(ServiceKey::from_cbor(&key.to_cbor()), Ok(key));
}

/// A field of a later version, key 8, a text string, is kept and encoded again as it came.
#[test]
fn a_key_secret_keeps_the_fields_of_later_versions() {
    let mut bytes = key().to_cbor();
    // A map of 7 entries, `0xa7`, becomes one of 8 with `8 => "x"` last.
    assert_eq!(bytes[0], 0xa7);
    bytes[0] = 0xa8;
    bytes.extend([0x08, 0x61, b'x']);
    let decoded = ServiceKey::from_cbor(&bytes).expect("a key secret");
    assert!(!decoded.unknown.is_empty());
    assert_eq!(decoded.to_cbor(), bytes);
}

/// A key secret without its anchor, or with a signing key of 31 bytes, does not decode.
#[test]
fn a_key_secret_without_a_field_or_with_a_short_key_does_not_decode() {
    let mut bytes = key().to_cbor();
    let anchor = [&[0x06, 0x58, 0x20][..], &[0x44; 32]].concat();
    let at = bytes
        .windows(anchor.len())
        .position(|window| window == anchor)
        .expect("the anchor");
    bytes.drain(at..at + anchor.len());
    bytes[0] = 0xa6;
    assert_field_problem(ServiceKey::from_cbor(&bytes), &FieldProblem::Missing);

    let mut bytes = key().to_cbor();
    let signing = [&[0x01, 0x58, 0x20][..], &[0x11; 32]].concat();
    let at = bytes
        .windows(signing.len())
        .position(|window| window == signing)
        .expect("the signing key");
    bytes.splice(
        at..at + signing.len(),
        [&[0x01, 0x58, 0x1f][..], &[0x11; 31]].concat(),
    );
    assert_field_problem(
        ServiceKey::from_cbor(&bytes),
        &FieldProblem::Length {
            expected: 32,
            actual: 31,
        },
    );
}

#[test]
fn a_key_secret_s_debug_form_shows_no_private_key() {
    let shown = format!("{:?}", key());
    assert!(shown.contains("skald.example.com"), "{shown}");
    for field in ["signing", "hpke"] {
        assert!(!shown.contains(field), "{shown}");
    }
}
