//! Device certificates: `docs/protocol/membership.md`, "Device certificate" and "Signed objects",
//! encoded as `docs/protocol/README.md`, "Encoding", says. A certificate keeps its body's bytes as
//! it got them. Malformed and truncated input is an error, never a panic.

use skald_proto::{
    AccountId, Cbor, Cert, CertBody, DecodeError, DeviceId, DeviceKind, FieldProblem, Unknown,
};

// A deterministic CBOR writer, independent of the crate's encoder.

fn head(major: u8, value: u64) -> Vec<u8> {
    let major = major << 5;
    let mut out = Vec::new();
    if let Ok(small @ 0..=23) = u8::try_from(value) {
        out.push(major | small);
    } else if let Ok(byte) = u8::try_from(value) {
        out.extend([major | 24, byte]);
    } else if let Ok(short) = u16::try_from(value) {
        out.push(major | 25);
        out.extend(short.to_be_bytes());
    } else if let Ok(word) = u32::try_from(value) {
        out.push(major | 26);
        out.extend(word.to_be_bytes());
    } else {
        out.push(major | 27);
        out.extend(value.to_be_bytes());
    }
    out
}

fn uint(value: u64) -> Vec<u8> {
    head(0, value)
}

fn int(value: i64) -> Vec<u8> {
    match u64::try_from(value) {
        Ok(positive) => uint(positive),
        Err(_) => head(1, (-1 - value).unsigned_abs()),
    }
}

fn bstr(value: &[u8]) -> Vec<u8> {
    let mut out = head(2, value.len() as u64);
    out.extend(value);
    out
}

fn tstr(value: &str) -> Vec<u8> {
    let mut out = head(3, value.len() as u64);
    out.extend(value.as_bytes());
    out
}

fn array(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = head(4, items.len() as u64);
    out.extend(items.concat());
    out
}

/// A map with integer keys in the deterministic order: bytewise by their encodings.
fn map(fields: Vec<(i64, Vec<u8>)>) -> Vec<u8> {
    let mut fields: Vec<(Vec<u8>, Vec<u8>)> = fields
        .into_iter()
        .map(|(key, value)| (int(key), value))
        .collect();
    fields.sort();
    raw_map(fields)
}

/// A map with these encoded keys and values, in the given order.
fn raw_map(fields: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    let mut out = head(5, fields.len() as u64);
    for (key, value) in fields {
        out.extend(key);
        out.extend(value);
    }
    out
}

// The values of the test certificate.

const SIGNING: [u8; 32] = [0x11; 32];
const HPKE: [u8; 32] = [0x22; 32];
const SSH: &[u8] = b"SSHSIG placeholder";
const ISSUED: u64 = 1_762_819_200;
const SIGNATURE: [u8; 64] = [0x5a; 64];
const PROOF: [u8; 64] = [0xa5; 64];

fn account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1".parse().unwrap()
}

fn device() -> DeviceId {
    DeviceId::from_signing_key(&SIGNING)
}

fn issuer() -> DeviceId {
    DeviceId::from_signing_key(&[0x33; 32])
}

/// A body with every field, the optional ones included.
fn full_body() -> CertBody {
    CertBody {
        account: account(),
        device: device(),
        kind: DeviceKind::Browser,
        signing: SIGNING,
        hpke: HPKE,
        issuer: Some(issuer()),
        issued: ISSUED,
        ssh: Some(SSH.to_vec()),
        name: "Phone".to_owned(),
        unknown: Unknown::default(),
    }
}

/// A body without the optional fields: one the master key issued, with no SSH signature.
fn minimal_body() -> CertBody {
    CertBody {
        kind: DeviceKind::Client,
        issuer: None,
        ssh: None,
        name: "Laptop".to_owned(),
        ..full_body()
    }
}

/// The fields of [`full_body`], encoded.
fn full_fields() -> Vec<(i64, Vec<u8>)> {
    vec![
        (1, bstr(account().as_bytes())),
        (2, bstr(device().as_bytes())),
        (3, uint(2)),
        (4, bstr(&SIGNING)),
        (5, bstr(&HPKE)),
        (6, bstr(issuer().as_bytes())),
        (7, uint(ISSUED)),
        (8, bstr(SSH)),
        (9, tstr("Phone")),
    ]
}

/// The fields of [`full_body`] with `key` replaced by `value`, or left out if `value` is `None`.
fn fields_with(key: i64, value: Option<Vec<u8>>) -> Vec<(i64, Vec<u8>)> {
    let mut fields: Vec<_> = full_fields()
        .into_iter()
        .filter(|(k, _)| *k != key)
        .collect();
    fields.extend(value.map(|value| (key, value)));
    fields
}

fn cert_bytes(body: &[u8]) -> Vec<u8> {
    array(vec![bstr(body), bstr(&SIGNATURE), bstr(&PROOF)])
}

fn assert_field_problem(result: Result<CertBody, DecodeError>, problem: &FieldProblem) {
    match result {
        Err(DecodeError::Field {
            problem: actual, ..
        }) => assert_eq!(&actual, problem),
        other => panic!("expected {problem:?}, got {other:?}"),
    }
}

#[test]
fn a_certificate_body_encodes_as_the_spec_says() {
    let bytes = map(full_fields());
    assert_eq!(full_body().to_cbor(), bytes);
    assert_eq!(CertBody::from_cbor(&bytes), Ok(full_body()));
}

#[test]
fn absent_optional_fields_are_left_out() {
    let bytes = map(vec![
        (1, bstr(account().as_bytes())),
        (2, bstr(device().as_bytes())),
        (3, uint(1)),
        (4, bstr(&SIGNING)),
        (5, bstr(&HPKE)),
        (7, uint(ISSUED)),
        (9, tstr("Laptop")),
    ]);
    assert_eq!(minimal_body().to_cbor(), bytes);
    assert_eq!(CertBody::from_cbor(&bytes), Ok(minimal_body()));
}

#[test]
fn device_kinds_have_the_spec_values() {
    for (kind, value) in [
        (DeviceKind::Client, 1),
        (DeviceKind::Browser, 2),
        (DeviceKind::Service, 3),
    ] {
        let bytes = map(fields_with(3, Some(uint(value))));
        assert_eq!(
            CertBody {
                kind,
                ..full_body()
            }
            .to_cbor(),
            bytes
        );
        assert_eq!(CertBody::from_cbor(&bytes).map(|body| body.kind), Ok(kind));
    }
}

#[test]
fn reserved_and_undefined_device_kinds_are_rejected() {
    for value in [0, 4, 5, 6, 255] {
        assert_field_problem(
            CertBody::from_cbor(&map(fields_with(3, Some(uint(value))))),
            &FieldProblem::Value(value),
        );
    }
    assert!(CertBody::from_cbor(&map(fields_with(3, Some(int(-1))))).is_err());
    assert!(CertBody::from_cbor(&map(fields_with(3, Some(tstr("client"))))).is_err());
}

#[test]
fn a_certificate_is_its_body_signature_and_proof() {
    let body = map(full_fields());
    let bytes = cert_bytes(&body);
    let cert = Cert::from_cbor(&bytes).unwrap();
    assert_eq!(cert.body(), &full_body());
    assert_eq!(cert.body_bytes(), body.as_slice());
    assert_eq!(cert.signature(), &SIGNATURE);
    assert_eq!(cert.proof(), &PROOF);
    assert_eq!(cert.to_cbor(), bytes);
}

#[test]
fn a_certificate_round_trips_byte_for_byte_with_unknown_fields() {
    let mut fields = full_fields();
    fields.extend([
        (10, tstr("a field of a later version")),
        (24, array(vec![uint(1), map(vec![(1, uint(2))])])),
        (1000, bstr(&[0xff; 3])),
        (-1, uint(0)),
        (-100, tstr("negative keys are integers too")),
    ]);
    let body = map(fields);
    let bytes = cert_bytes(&body);

    let cert = Cert::from_cbor(&bytes).unwrap();
    assert_eq!(cert.to_cbor(), bytes);
    assert_eq!(cert.body_bytes(), body.as_slice());
    let mut keys: Vec<i128> = cert.body().unknown.keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, [-100, -1, 10, 24, 1000]);
    assert_eq!(
        cert.body().unknown.get(10),
        Some(tstr("a field of a later version").as_slice())
    );
    assert_eq!(
        CertBody {
            unknown: Unknown::default(),
            ..cert.body().clone()
        },
        full_body()
    );

    let decoded = CertBody::from_cbor(&body).unwrap();
    assert_eq!(decoded.to_cbor(), body);
    assert_eq!(&decoded, cert.body());
}

#[test]
fn a_minimal_certificate_round_trips_byte_for_byte() {
    let body = minimal_body().to_cbor();
    let bytes = cert_bytes(&body);
    let cert = Cert::from_cbor(&bytes).unwrap();
    assert_eq!(cert.body(), &minimal_body());
    assert_eq!(cert.to_cbor(), bytes);
}

#[test]
fn required_fields_are_required() {
    for key in [1, 2, 3, 4, 5, 7, 9] {
        let body = map(fields_with(key, None));
        assert_field_problem(CertBody::from_cbor(&body), &FieldProblem::Missing);
        assert!(Cert::from_cbor(&cert_bytes(&body)).is_err(), "key {key}");
    }
}

#[test]
fn fields_of_another_size_are_rejected() {
    for (key, value) in [
        (1, bstr(&[1; 15])),
        (1, bstr(&[1; 17])),
        (2, bstr(&[2; 15])),
        (2, bstr(&[2; 17])),
        (4, bstr(&[4; 31])),
        (4, bstr(&[4; 33])),
        (5, bstr(&[5; 31])),
        (5, bstr(&[5; 33])),
        (6, bstr(&[6; 15])),
        (6, bstr(&[6; 17])),
    ] {
        let body = map(fields_with(key, Some(value)));
        assert!(CertBody::from_cbor(&body).is_err(), "key {key}");
        assert!(Cert::from_cbor(&cert_bytes(&body)).is_err(), "key {key}");
    }
}

#[test]
fn fields_of_another_type_are_rejected() {
    for (key, value) in [
        (1, tstr("019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1")),
        (2, uint(2)),
        (4, array(vec![])),
        (5, tstr("hpke")),
        (6, uint(6)),
        (7, int(-1)),
        (7, bstr(&ISSUED.to_be_bytes())),
        (8, tstr("ssh")),
        (9, bstr(b"Phone")),
    ] {
        let body = map(fields_with(key, Some(value)));
        assert!(
            matches!(
                CertBody::from_cbor(&body),
                Err(DecodeError::Field {
                    problem: FieldProblem::Type(_),
                    ..
                })
            ),
            "key {key}"
        );
        assert!(Cert::from_cbor(&cert_bytes(&body)).is_err(), "key {key}");
    }
}

#[test]
fn a_name_is_1_to_64_bytes_of_text() {
    for name in ["a", &"a".repeat(64), &"é".repeat(32)] {
        let body = CertBody::from_cbor(&map(fields_with(9, Some(tstr(name))))).unwrap();
        assert_eq!(body.name, name);
    }
    for name in ["", &"a".repeat(65), &"é".repeat(33)] {
        let body = map(fields_with(9, Some(tstr(name))));
        assert!(CertBody::from_cbor(&body).is_err(), "{} bytes", name.len());
        assert!(Cert::from_cbor(&cert_bytes(&body)).is_err());
    }
}

#[test]
fn a_certificate_is_an_array_of_three_byte_strings() {
    let body = bstr(&map(full_fields()));
    for bytes in [
        array(vec![]),
        array(vec![body.clone()]),
        array(vec![body.clone(), bstr(&SIGNATURE)]),
        array(vec![
            body.clone(),
            bstr(&SIGNATURE),
            bstr(&PROOF),
            bstr(&PROOF),
        ]),
        map(vec![
            (0, body.clone()),
            (1, bstr(&SIGNATURE)),
            (2, bstr(&PROOF)),
        ]),
        bstr(&map(full_fields())),
        // The body as a map instead of a byte string that holds one.
        array(vec![map(full_fields()), bstr(&SIGNATURE), bstr(&PROOF)]),
        array(vec![body.clone(), tstr("signature"), bstr(&PROOF)]),
        array(vec![body.clone(), bstr(&SIGNATURE), uint(0)]),
    ] {
        assert!(Cert::from_cbor(&bytes).is_err(), "{bytes:02x?}");
    }
}

#[test]
fn signatures_are_64_bytes() {
    let body = bstr(&map(full_fields()));
    for (signature, proof) in [
        (&SIGNATURE[..63], &PROOF[..]),
        (&[0x5a; 65][..], &PROOF[..]),
        (&[][..], &PROOF[..]),
        (&SIGNATURE[..], &PROOF[..63]),
        (&SIGNATURE[..], &[0xa5; 65][..]),
        (&SIGNATURE[..], &[][..]),
    ] {
        let bytes = array(vec![body.clone(), bstr(signature), bstr(proof)]);
        assert!(Cert::from_cbor(&bytes).is_err());
    }
}

#[test]
fn the_body_holds_exactly_one_cert_body() {
    let body = map(full_fields());
    let mut trailing = body.clone();
    trailing.push(0x00);
    for inner in [
        trailing,
        body[..body.len() - 1].to_vec(),
        Vec::new(),
        uint(1),
        array(vec![bstr(&body)]),
        vec![0xff],
    ] {
        assert!(
            Cert::from_cbor(&cert_bytes(&inner)).is_err(),
            "{inner:02x?}"
        );
    }
}

#[test]
fn a_non_deterministic_body_is_rejected() {
    let fields: Vec<(Vec<u8>, Vec<u8>)> = full_fields()
        .into_iter()
        .map(|(key, value)| (int(key), value))
        .collect();
    let mut unsorted = fields.clone();
    unsorted.swap(0, 1);
    let mut repeated = fields.clone();
    repeated.insert(1, fields[0].clone());
    let mut long_head = fields.clone();
    long_head[6].1 = [vec![0x1b], ISSUED.to_be_bytes().to_vec()].concat();
    let mut text_key = fields.clone();
    text_key.push((tstr("name"), tstr("Phone")));
    // An indefinite-length map.
    let mut indefinite = vec![0xbf];
    for (key, value) in &fields {
        indefinite.extend(key);
        indefinite.extend(value);
    }
    indefinite.push(0xff);

    for body in [
        raw_map(unsorted),
        raw_map(repeated),
        raw_map(long_head),
        raw_map(text_key),
        indefinite,
    ] {
        assert!(CertBody::from_cbor(&body).is_err(), "{body:02x?}");
        assert!(Cert::from_cbor(&cert_bytes(&body)).is_err(), "{body:02x?}");
    }
}

#[test]
fn a_non_deterministic_certificate_is_rejected() {
    let body = map(full_fields());
    // The body's byte string with a two-byte length head where one byte is enough.
    let mut long_head = vec![0x83, 0x59];
    long_head.extend(u16::try_from(body.len()).unwrap().to_be_bytes());
    long_head.extend(&body);
    long_head.extend(bstr(&SIGNATURE));
    long_head.extend(bstr(&PROOF));
    assert!(body.len() < 256);
    assert!(Cert::from_cbor(&long_head).is_err());

    // An indefinite-length array.
    let mut indefinite = vec![0x9f];
    indefinite.extend(bstr(&body));
    indefinite.extend(bstr(&SIGNATURE));
    indefinite.extend(bstr(&PROOF));
    indefinite.push(0xff);
    assert!(Cert::from_cbor(&indefinite).is_err());

    // A tagged certificate.
    let mut tagged = vec![0xd8, 0x18];
    tagged.extend(cert_bytes(&body));
    assert!(Cert::from_cbor(&tagged).is_err());
}

#[test]
fn truncated_certificates_are_rejected() {
    let bytes = cert_bytes(&map(full_fields()));
    for len in 0..bytes.len() {
        assert!(Cert::from_cbor(&bytes[..len]).is_err(), "{len} bytes");
    }
    let body = map(full_fields());
    for len in 0..body.len() {
        assert!(CertBody::from_cbor(&body[..len]).is_err(), "{len} bytes");
    }
}

#[test]
fn bytes_after_a_certificate_are_rejected() {
    let mut bytes = cert_bytes(&map(full_fields()));
    bytes.push(0x00);
    assert_eq!(Cert::from_cbor(&bytes), Err(DecodeError::TrailingBytes(1)));
}

#[test]
fn corrupted_certificates_never_panic_and_re_encode_as_received() {
    let mut fields = full_fields();
    fields.push((10, tstr("later")));
    let body = map(fields);
    let bytes = cert_bytes(&body);
    for at in 0..bytes.len() {
        for flip in [0x01, 0x02, 0x10, 0x20, 0x40, 0x80, 0xff] {
            let mut corrupted = bytes.clone();
            corrupted[at] ^= flip;
            if let Ok(cert) = Cert::from_cbor(&corrupted) {
                assert_eq!(cert.to_cbor(), corrupted, "byte {at} ^ {flip:#04x}");
            }
        }
    }
    for at in 0..body.len() {
        for flip in [0x01, 0x02, 0x10, 0x20, 0x40, 0x80, 0xff] {
            let mut corrupted = body.clone();
            corrupted[at] ^= flip;
            if let Ok(decoded) = CertBody::from_cbor(&corrupted) {
                assert_eq!(decoded.to_cbor(), corrupted, "byte {at} ^ {flip:#04x}");
            }
        }
    }
}
