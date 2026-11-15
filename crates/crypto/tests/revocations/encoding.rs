//! The revocation type of `skald-proto`: `membership.md`, "Revocation", encoded as "Signed objects"
//! and `README.md`, "Encoding", say. A revocation keeps its body's bytes as it got them, unknown
//! fields included. Malformed and truncated input is refused, never a panic.

use skald_proto::{Cbor, DecodeError, Revocation, RevocationBody};

use crate::cbor::{Value, bytes, text};
use crate::fields::{assert_signed_round_trip, signed_cbor, unknown};
use crate::fixtures::*;

/// A revocation by the master key with every optional field and fields of later versions.
fn rich() -> Rev {
    Rev {
        extra: unknown(),
        ..Rev::by_master(DESKTOP)
            .keep(&[PHONE, TABLET])
            .also(&[BROWSER])
    }
}

/// A revocation by a device that keeps a device, with fields of later versions.
fn by_device() -> Rev {
    Rev {
        extra: unknown(),
        ..Rev::by(BROWSER, DESKTOP).keep(&[PHONE])
    }
}

/// The fields of `rev`'s body.
fn fields_of(rev: &Rev) -> Vec<(i64, Value)> {
    match rev.value() {
        Value::Map(fields) => fields,
        other => panic!("not a map: {other:?}"),
    }
}

/// The encoded map of these entries in this order, each value already encoded: no sorting.
fn raw_map(entries: &[(u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0xa0 | u8::try_from(entries.len()).unwrap()];
    for (key, value) in entries {
        assert!(*key < 24);
        out.push(*key);
        out.extend(value);
    }
    out
}

/// The body of [`Rev::by`] the laptop of the desktop, its fields in order, each encoded.
fn plain_entries() -> Vec<(u8, Vec<u8>)> {
    vec![
        (1, bytes(ACCOUNT.as_bytes()).encode()),
        (2, bytes(device(DESKTOP).as_bytes()).encode()),
        (4, bytes(device(LAPTOP).as_bytes()).encode()),
        (5, Value::Uint(TIMESTAMP).encode()),
    ]
}

/// Checks that `body`, described as `what`, decodes neither alone nor in a revocation.
fn assert_refused(what: &str, body: &[u8]) {
    assert!(RevocationBody::from_cbor(body).is_err(), "{what}");
    assert!(
        Revocation::from_cbor(&signed_cbor(body, &[0x5a; 64])).is_err(),
        "a revocation with {what}"
    );
}

fn ids(seeds: &[u8]) -> Vec<skald_proto::DeviceId> {
    seeds.iter().copied().map(device).collect()
}

/// Checks that `rev` decodes into its spec fields and its signature.
fn assert_decodes(rev: &Rev) {
    let revocation = Revocation::from_cbor(&rev.bytes()).unwrap();
    let body = revocation.body();
    let list = |seeds: &[u8]| (!seeds.is_empty()).then(|| ids(seeds));
    assert_eq!(body.account, rev.account);
    assert_eq!(body.device, device(rev.device));
    assert_eq!(body.keep.as_deref(), list(&rev.keep).as_deref());
    assert_eq!(body.signer, rev.signer.map(device));
    assert_eq!(body.timestamp, TIMESTAMP);
    assert_eq!(body.also.as_deref(), list(&rev.also).as_deref());
    assert_eq!(revocation.signature(), &rev.signature());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_decodes_into_its_spec_fields() {
    assert_decodes(&rich());
    assert_decodes(&by_device());
    assert_decodes(&Rev::by(LAPTOP, DESKTOP));
    // `also` with a `signer` decodes; checking it ignores it.
    assert_decodes(&Rev::by(LAPTOP, DESKTOP).also(&[TABLET]));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_round_trips_byte_for_byte_with_its_unknown_fields() {
    for rev in [rich(), by_device()] {
        let body = RevocationBody::from_cbor(&rev.body()).unwrap();
        let mut keys: Vec<i128> = body.unknown.keys().collect();
        keys.sort_unstable();
        assert_eq!(keys, [-3, 24]);
        assert_eq!(body.to_cbor(), rev.body(), "the body, byte for byte");
        assert_signed_round_trip(&rev.body(), &body);

        let revocation = Revocation::from_cbor(&rev.bytes()).unwrap();
        assert_eq!(revocation.body(), &body);
        assert_eq!(revocation.body_bytes(), rev.body());
        assert_eq!(
            revocation.to_cbor(),
            rev.bytes(),
            "the revocation, byte for byte"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_without_optional_fields_round_trips() {
    let rev = Rev::by_master(TABLET);
    let body = RevocationBody::from_cbor(&rev.body()).unwrap();
    assert_eq!(body.keep, None);
    assert_eq!(body.signer, None);
    assert_eq!(body.also, None);
    assert!(body.unknown.is_empty());
    assert_eq!(body.to_cbor(), rev.body());
    assert_eq!(
        Revocation::from_cbor(&rev.bytes()).unwrap().to_cbor(),
        rev.bytes()
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_truncation_of_a_revocation_is_refused() {
    let encoded = rich().bytes();
    let not_truncated: Vec<usize> = (0..encoded.len())
        .filter(|&len| Revocation::from_cbor(&encoded[..len]) != Err(DecodeError::Truncated))
        .collect();
    assert!(
        not_truncated.is_empty(),
        "prefixes not refused as truncated: {not_truncated:?}"
    );

    let body = rich().body();
    let not_truncated: Vec<usize> = (0..body.len())
        .filter(|&len| RevocationBody::from_cbor(&body[..len]) != Err(DecodeError::Truncated))
        .collect();
    assert!(
        not_truncated.is_empty(),
        "body prefixes not refused as truncated: {not_truncated:?}"
    );
    let decoded: Vec<usize> = (0..body.len())
        .filter(|&len| Revocation::from_cbor(&signed_cbor(&body[..len], &[0x5a; 64])).is_ok())
        .collect();
    assert!(
        decoded.is_empty(),
        "revocations of body prefixes that decode: {decoded:?}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_with_bytes_after_it_is_refused() {
    let mut longer = rich().bytes();
    longer.push(0);
    assert_eq!(
        Revocation::from_cbor(&longer),
        Err(DecodeError::TrailingBytes(1))
    );
    let mut body = rich().body();
    body.extend([0x01, 0x02]);
    assert_eq!(
        RevocationBody::from_cbor(&body),
        Err(DecodeError::TrailingBytes(2))
    );
    assert!(Revocation::from_cbor(&signed_cbor(&body, &[0x5a; 64])).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_whose_body_breaks_the_cddl_does_not_decode() {
    let fields = fields_of(&rich());
    let set = |key: i64, value: Value| crate::fields::with(&fields, key, value);
    let id15 = || bytes(&[0x5a; 15]);
    let mut broken: Vec<(&str, Vec<u8>)> = vec![
        ("without account", crate::fields::without(&fields, 1)),
        ("without device", crate::fields::without(&fields, 2)),
        ("without timestamp", crate::fields::without(&fields, 5)),
        ("a 15-byte account", set(1, id15())),
        ("a 17-byte account", set(1, bytes(&[0x5a; 17]))),
        ("an account of text", set(1, text("account"))),
        ("a 15-byte device", set(2, id15())),
        ("a device of a number", set(2, Value::Uint(7))),
        ("an empty keep", set(3, Value::Array(vec![]))),
        ("a keep of bytes", set(3, bytes(device(PHONE).as_bytes()))),
        (
            "a keep with a 15-byte item",
            set(3, Value::Array(vec![id15()])),
        ),
        (
            "a keep with a text item",
            set(3, Value::Array(vec![text("phone")])),
        ),
        ("a 17-byte signer", set(4, bytes(&[0x5a; 17]))),
        (
            "a signer in an array",
            set(4, Value::Array(vec![bytes(device(LAPTOP).as_bytes())])),
        ),
        ("a timestamp of text", set(5, text("1800000000"))),
        (
            "a timestamp of bytes",
            set(5, bytes(&[0x6b, 0x49, 0xd2, 0x00])),
        ),
        ("an empty also", set(6, Value::Array(vec![]))),
        (
            "an also with a 15-byte item",
            set(6, Value::Array(vec![id15()])),
        ),
        ("an also of a number", set(6, Value::Uint(1))),
        (
            "an array, not a map",
            Value::Array(vec![bytes(ACCOUNT.as_bytes())]).encode(),
        ),
        ("a number, not a map", Value::Uint(1).encode()),
        ("an empty map", Value::Map(vec![]).encode()),
    ];
    // A negative timestamp: major type 1.
    let mut negative = plain_entries();
    negative[3].1 = vec![0x20];
    broken.push(("a negative timestamp", raw_map(&negative)));

    for (what, body) in broken {
        assert_refused(what, &body);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_that_is_not_a_signed_object_does_not_decode() {
    let body = rich().body();
    let signature = rich().signature();
    let broken: Vec<(&str, Vec<u8>)> = vec![
        ("no signature", Value::Array(vec![bytes(&body)]).encode()),
        (
            "a third item",
            Value::Array(vec![bytes(&body), bytes(&signature), bytes(&signature)]).encode(),
        ),
        ("a 63-byte signature", signed_cbor(&body, &signature[..63])),
        (
            "a 65-byte signature",
            signed_cbor(&body, &[signature.as_slice(), &[0]].concat()),
        ),
        (
            "a body of text",
            Value::Array(vec![text("body"), bytes(&signature)]).encode(),
        ),
        (
            "a body not in a byte string",
            Value::Array(vec![rich().value(), bytes(&signature)]).encode(),
        ),
        ("a map, not an array", rich().body()),
        ("an empty array", Value::Array(vec![]).encode()),
    ];
    for (what, encoded) in broken {
        assert!(Revocation::from_cbor(&encoded).is_err(), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revocation_that_breaks_the_encoding_rules_is_refused() {
    let entries = plain_entries();
    // The plain entries encode as the writer encodes the revocation.
    assert_eq!(raw_map(&entries), Rev::by(LAPTOP, DESKTOP).body());

    let mut unsorted = entries.clone();
    unsorted.swap(0, 1);
    let mut repeated = entries.clone();
    repeated.insert(1, entries[0].clone());
    // The timestamp in a head of 8 bytes, not its shortest of 4.
    let mut long_head = entries.clone();
    long_head[3].1 = [vec![0x1b], TIMESTAMP.to_be_bytes().to_vec()].concat();
    // `keep` as an indefinite-length array.
    let mut indefinite = entries.clone();
    let phone = bytes(device(PHONE).as_bytes()).encode();
    indefinite.insert(2, (3, [vec![0x9f], phone, vec![0xff]].concat()));

    for (what, body) in [
        ("unsorted keys", raw_map(&unsorted)),
        ("a repeated key", raw_map(&repeated)),
        ("a head longer than its shortest form", raw_map(&long_head)),
        ("an indefinite-length array", raw_map(&indefinite)),
    ] {
        assert_refused(what, &body);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_corrupted_revocation_is_refused_or_decodes_to_what_it_encodes() {
    let encoded = rich().bytes();
    for at in 0..encoded.len() {
        for flip in [0x01, 0x08, 0x20, 0x80, 0xff] {
            let mut corrupted = encoded.clone();
            corrupted[at] ^= flip;
            if let Ok(revocation) = Revocation::from_cbor(&corrupted) {
                assert_eq!(revocation.to_cbor(), corrupted, "byte {at} ^ {flip:#04x}");
            }
        }
    }
}
