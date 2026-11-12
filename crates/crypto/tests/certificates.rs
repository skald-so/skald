//! Ed25519 signatures over labels (`docs/protocol/README.md`, "Conventions"; `membership.md`,
//! "Signed objects"), issuing device certificates (`membership.md`, "Device certificate",
//! "Linking a device") and checking them against a pinned master key (`membership.md`,
//! "Device checks", checks 1 and 2).
//!
//! The expected signatures are made with ed25519-dalek over the encodings the spec gives, apart
//! from the crate under test: Ed25519 is deterministic, so they must match byte for byte.

use ed25519_dalek::Signer as _;
use skald_crypto::{DeviceCheckError, SigningKey, check_device, issue, verify};
use skald_proto::{AccountId, Cbor, Cert, CertBody, DeviceId, DeviceKind, Unknown};

// Key seeds.
const MASTER: u8 = 1;
const OTHER_MASTER: u8 = 2;
const SERVER: u8 = 3;
const A: u8 = 10;
const B: u8 = 11;
const C: u8 = 12;
const D: u8 = 13;

const CERT: &str = "skald cert";
const PROOF: &str = "skald proof";

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn dalek(seed: u8) -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[seed; 32])
}

fn public(seed: u8) -> [u8; 32] {
    dalek(seed).verifying_key().to_bytes()
}

fn id(seed: u8) -> DeviceId {
    DeviceId::from_signing_key(&public(seed))
}

/// 32 bytes from 64 hex digits.
fn bytes32(hex: &str) -> [u8; 32] {
    let mut out = [0; 32];
    for (byte, at) in out.iter_mut().zip((0..64).step_by(2)) {
        *byte = u8::from_str_radix(&hex[at..at + 2], 16).unwrap();
    }
    out
}

/// The CBOR head of a text or byte string of at most 255 bytes.
fn string_head(major: u8, len: usize) -> Vec<u8> {
    let len = u8::try_from(len).unwrap();
    if len < 24 {
        vec![(major << 5) | len]
    } else {
        vec![(major << 5) | 24, len]
    }
}

/// The encoded `[label, parts...]`, the parts as byte strings; each part is at most 255 bytes.
fn signed_data(label: &str, parts: &[&[u8]]) -> Vec<u8> {
    let mut out = vec![0x80 | u8::try_from(1 + parts.len()).unwrap()];
    out.extend(string_head(3, label.len()));
    out.extend(label.as_bytes());
    for part in parts {
        out.extend(string_head(2, part.len()));
        out.extend(*part);
    }
    out
}

/// The signature of `seed`'s key over `[label, body]`, made apart from the crate under test.
fn signature(seed: u8, label: &str, body: &[u8]) -> [u8; 64] {
    dalek(seed).sign(&signed_data(label, &[body])).to_bytes()
}

/// The encoded `cert`: `[body, signature, proof]`.
fn cert_bytes(body: &[u8], signature: &[u8; 64], proof: &[u8; 64]) -> Vec<u8> {
    let mut out = vec![0x83];
    out.extend(string_head(2, body.len()));
    out.extend(body);
    for signature in [signature, proof] {
        out.extend([0x58, 64]);
        out.extend(signature);
    }
    out
}

fn cert_of(body: &[u8], signature: &[u8; 64], proof: &[u8; 64]) -> Cert {
    Cert::from_cbor(&cert_bytes(body, signature, proof)).unwrap()
}

fn account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc1".parse().unwrap()
}

fn other_account() -> AccountId {
    "019a6f1e-2b3c-7d4e-8f50-61a2b3c4acc2".parse().unwrap()
}

/// The body of `device`'s certificate on [`account`], issued by `issuer` or, if `None`, by the
/// master key.
fn body(device: u8, kind: DeviceKind, issuer: Option<u8>) -> CertBody {
    CertBody {
        account: account(),
        device: id(device),
        kind,
        signing: public(device),
        hpke: [device; 32],
        issuer: issuer.map(id),
        issued: 1_762_819_200,
        ssh: None,
        name: format!("Device {device}"),
        unknown: Unknown::default(),
    }
}

/// `body` signed by `signer` and proved by `prover` over their labels, apart from [`issue`].
fn signed(body: &CertBody, signer: u8, prover: u8) -> Cert {
    let bytes = body.to_cbor();
    cert_of(
        &bytes,
        &signature(signer, CERT, &bytes),
        &signature(prover, PROOF, &bytes),
    )
}

/// The certificate of a `client` device issued by the master key.
fn first(device: u8) -> Cert {
    signed(&body(device, DeviceKind::Client, None), MASTER, device)
}

/// The certificate of `device` of `kind` issued by `issuer`.
fn linked(device: u8, kind: DeviceKind, issuer: u8) -> Cert {
    signed(&body(device, kind, Some(issuer)), issuer, device)
}

/// `cert` with one bit of its signature flipped.
fn broken_signature(cert: &Cert) -> Cert {
    let mut signature = *cert.signature();
    signature[17] ^= 0x01;
    cert_of(cert.body_bytes(), &signature, cert.proof())
}

/// `cert` with one bit of its proof flipped.
fn broken_proof(cert: &Cert) -> Cert {
    let mut proof = *cert.proof();
    proof[42] ^= 0x80;
    cert_of(cert.body_bytes(), cert.signature(), &proof)
}

fn check(certs: &[Cert], device: u8) -> Result<&Cert, DeviceCheckError> {
    check_device(&public(MASTER), certs, id(device))
}

#[track_caller]
fn assert_valid(certs: &[Cert], device: u8, expected: &Cert) {
    match check(certs, device) {
        Ok(cert) => assert_eq!(cert, expected),
        Err(error) => panic!("device {device}: {error:?}"),
    }
}

/// Not valid, for a reason other than two bodies.
#[track_caller]
fn assert_invalid(certs: &[Cert], device: u8) {
    match check(certs, device) {
        Err(DeviceCheckError::TwoBodies) => panic!("device {device}: two bodies reported"),
        Err(_) => {}
        Ok(cert) => panic!("device {device}: valid with {cert:?}"),
    }
}

#[track_caller]
fn assert_two_bodies(certs: &[Cert], device: u8) {
    match check(certs, device) {
        Err(DeviceCheckError::TwoBodies) => {}
        other => panic!("device {device}: expected two bodies, got {other:?}"),
    }
}

// Signatures.

#[test]
fn a_signing_key_has_the_rfc_8032_public_key() {
    // RFC 8032, §7.1, "TEST 1" and "TEST 2".
    for (secret, expected) in [
        (
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        ),
        (
            "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        ),
    ] {
        let (secret, expected) = (bytes32(secret), bytes32(expected));
        assert_eq!(SigningKey::from_bytes(&secret).public_key(), expected);
    }
    assert_eq!(key(A).public_key(), public(A));
}

#[test]
fn a_signature_covers_the_encoded_label_and_byte_strings() {
    let body = b"a body".as_slice();
    assert_eq!(key(A).sign(CERT, &[body]), signature(A, CERT, body));
    let (header, content) = (b"header".as_slice(), [0x42; 100]);
    assert_eq!(
        key(B).sign("skald hpke", &[header, &content]),
        dalek(B)
            .sign(&signed_data("skald hpke", &[header, &content]))
            .to_bytes()
    );
}

#[test]
fn a_signature_verifies_only_with_its_key_label_and_bytes() {
    let parts: [&[u8]; 2] = [b"header", b"content"];
    let made = key(A).sign("skald hpke", &parts);
    assert!(verify(&public(A), "skald hpke", &parts, &made).is_ok());

    assert!(verify(&public(B), "skald hpke", &parts, &made).is_err());
    assert!(verify(&public(A), "skald grant", &parts, &made).is_err());
    assert!(
        verify(
            &public(A),
            "skald hpke",
            &[b"header".as_slice(), b"contenu"],
            &made
        )
        .is_err()
    );
    assert!(verify(&public(A), "skald hpke", &[b"header".as_slice()], &made).is_err());
    assert!(
        verify(
            &public(A),
            "skald hpke",
            &[b"header".as_slice(), b"content", b""],
            &made
        )
        .is_err()
    );
    let mut flipped = made;
    flipped[0] ^= 0x01;
    assert!(verify(&public(A), "skald hpke", &parts, &flipped).is_err());
    // Other bytes as a public key verify nothing and do not panic.
    assert!(verify(&[0xff; 32], "skald hpke", &parts, &made).is_err());
}

#[test]
fn generated_keys_differ_and_sign() {
    let (first, second) = (SigningKey::generate(), SigningKey::generate());
    assert_ne!(first.public_key(), second.public_key());
    let made = first.sign(PROOF, &[b"body"]);
    assert!(verify(&first.public_key(), PROOF, &[b"body"], &made).is_ok());
    assert!(verify(&second.public_key(), PROOF, &[b"body"], &made).is_err());
}

// Issuing.

#[test]
fn the_master_key_signs_and_the_device_proves_over_their_labels() {
    let body = body(A, DeviceKind::Client, None);
    let cert = issue(&body, &key(MASTER), &key(A));
    let bytes = body.to_cbor();
    assert_eq!(cert.body(), &body);
    assert_eq!(cert.body_bytes(), bytes.as_slice());
    assert_eq!(cert.signature(), &signature(MASTER, CERT, &bytes));
    assert_eq!(cert.proof(), &signature(A, PROOF, &bytes));
    assert_eq!(cert, first(A));
    assert_valid(std::slice::from_ref(&cert), A, &cert);
}

#[test]
fn an_issuer_device_signs_and_the_device_proves_over_their_labels() {
    let body = body(B, DeviceKind::Browser, Some(A));
    let cert = issue(&body, &key(A), &key(B));
    let bytes = body.to_cbor();
    assert_eq!(cert.signature(), &signature(A, CERT, &bytes));
    assert_eq!(cert.proof(), &signature(B, PROOF, &bytes));
    assert_eq!(cert, linked(B, DeviceKind::Browser, A));
    assert_valid(&[first(A), cert.clone()], B, &cert);
}

// Valid certificates.

#[test]
fn a_certificate_signed_by_the_master_key_is_valid() {
    assert_valid(&[first(A)], A, &first(A));
}

#[test]
fn a_certificate_chained_to_the_master_key_is_valid() {
    let b = linked(B, DeviceKind::Client, A);
    let c = linked(C, DeviceKind::Service, B);
    // The order of the server's list does not matter.
    for certs in [
        [first(A), b.clone(), c.clone()],
        [c.clone(), b.clone(), first(A)],
        [b.clone(), c.clone(), first(A)],
    ] {
        assert_valid(&certs, A, &first(A));
        assert_valid(&certs, B, &b);
        assert_valid(&certs, C, &c);
    }
}

#[test]
fn a_client_device_issues_every_kind() {
    for kind in [DeviceKind::Client, DeviceKind::Browser, DeviceKind::Service] {
        let b = linked(B, kind, A);
        assert_valid(&[first(A), b.clone()], B, &b);
    }
}

#[test]
fn a_body_is_checked_as_the_bytes_it_came_in() {
    // A body with a field of a later version: key 10, after the known keys 1 to 9.
    let mut bytes = body(A, DeviceKind::Client, None).to_cbor();
    assert_eq!(bytes[0], 0xa7, "a map of 7 fields");
    bytes[0] = 0xa8;
    bytes.extend([0x0a, 0x63, b'n', b'e', b'w']);
    let cert = cert_of(
        &bytes,
        &signature(MASTER, CERT, &bytes),
        &signature(A, PROOF, &bytes),
    );
    assert_eq!(cert.body_bytes(), bytes.as_slice());
    assert_valid(std::slice::from_ref(&cert), A, &cert);
    assert_eq!(
        check(std::slice::from_ref(&cert), A).unwrap().to_cbor(),
        cert_bytes(&bytes, cert.signature(), cert.proof())
    );

    let b = linked(B, DeviceKind::Client, A);
    assert_valid(&[cert, b.clone()], B, &b);
}

// The proof check.

#[test]
fn a_device_id_not_derived_from_the_signing_key_is_not_valid() {
    // A's keys under B's ID, proved and signed with the right keys.
    let forged = CertBody {
        device: id(B),
        ..body(A, DeviceKind::Client, None)
    };
    assert_invalid(&[signed(&forged, MASTER, A)], B);
    assert_invalid(&[signed(&forged, MASTER, A)], A);
    // An ID that no key derives.
    let random = CertBody {
        device: DeviceId::from_bytes([0x5a; 16]),
        ..body(A, DeviceKind::Client, None)
    };
    let certs = [signed(&random, MASTER, A)];
    assert!(check_device(&public(MASTER), &certs, DeviceId::from_bytes([0x5a; 16])).is_err());
}

#[test]
fn only_the_holder_of_a_signing_key_certifies_its_device_id() {
    // Another key under B's ID beside B's genuine certificate: discarded, so B has one body.
    let forged = CertBody {
        device: id(B),
        ..body(C, DeviceKind::Client, None)
    };
    let certs = [signed(&forged, MASTER, C), first(B)];
    assert_valid(&certs, B, &first(B));
    let certs = [first(B), signed(&forged, MASTER, B)];
    assert_valid(&certs, B, &first(B));
}

#[test]
fn a_broken_proof_is_not_valid() {
    assert_invalid(&[broken_proof(&first(A))], A);
    // A proof by another key.
    assert_invalid(&[signed(&body(A, DeviceKind::Client, None), MASTER, B)], A);
    // A proof by the master key.
    assert_invalid(
        &[signed(&body(A, DeviceKind::Client, None), MASTER, MASTER)],
        A,
    );
}

// Check 1: the signature.

#[test]
fn a_broken_signature_is_not_valid() {
    assert_invalid(&[broken_signature(&first(A))], A);
    let b = linked(B, DeviceKind::Client, A);
    assert_invalid(&[first(A), broken_signature(&b)], B);
}

#[test]
fn a_signature_by_another_key_is_not_valid() {
    let a = body(A, DeviceKind::Client, None);
    // A server's signature alone never admits a device.
    assert_invalid(&[signed(&a, SERVER, A)], A);
    assert_invalid(&[signed(&a, OTHER_MASTER, A)], A);
    // Self-signed, with no issuer.
    assert_invalid(&[signed(&a, A, A)], A);
    // Signed by a device other than the issuer it names.
    let b = signed(&body(B, DeviceKind::Client, Some(A)), C, B);
    assert_invalid(&[first(A), first(C), b], B);
    // Naming an issuer, but signed by the master key.
    let b = signed(&body(B, DeviceKind::Client, Some(A)), MASTER, B);
    assert_invalid(&[first(A), b], B);
}

#[test]
fn a_signature_over_another_label_is_not_valid() {
    let bytes = body(A, DeviceKind::Client, None).to_cbor();
    let proof = signature(A, PROOF, &bytes);
    for label in [
        PROOF,
        "skald account",
        "skald link",
        "skald revocation",
        "skald certs",
        "",
    ] {
        let cert = cert_of(&bytes, &signature(MASTER, label, &bytes), &proof);
        assert_invalid(&[cert], A);
    }
    // Over the body alone, with no label.
    let bare = dalek(MASTER).sign(&bytes).to_bytes();
    assert_invalid(&[cert_of(&bytes, &bare, &proof)], A);
    // A proof over another label.
    let signature_ok = signature(MASTER, CERT, &bytes);
    for label in [CERT, "skald link", "skald prooff"] {
        let cert = cert_of(&bytes, &signature_ok, &signature(A, label, &bytes));
        assert_invalid(&[cert], A);
    }
    // The signature and the proof swapped.
    assert_invalid(&[cert_of(&bytes, &proof, &signature_ok)], A);
}

// Check 2: the chain.

#[test]
fn a_chain_to_another_master_key_is_not_valid() {
    let a = signed(&body(A, DeviceKind::Client, None), OTHER_MASTER, A);
    let b = linked(B, DeviceKind::Client, A);
    let certs = [a.clone(), b.clone()];
    assert_invalid(&certs, A);
    assert_invalid(&certs, B);
    // The same chain is valid against the master key it ends at.
    assert!(matches!(check_device(&public(OTHER_MASTER), &certs, id(B)), Ok(cert) if *cert == b));
    assert!(matches!(check_device(&public(OTHER_MASTER), &certs, id(A)), Ok(cert) if *cert == a));
}

#[test]
fn a_chain_that_repeats_a_device_is_not_valid() {
    // Issued by itself.
    assert_invalid(&[linked(A, DeviceKind::Client, A)], A);
    // A and B issued by each other.
    let certs = [
        linked(A, DeviceKind::Client, B),
        linked(B, DeviceKind::Client, A),
    ];
    assert_invalid(&certs, A);
    assert_invalid(&certs, B);
    // A longer loop, hanging off a device that is valid on its own.
    let certs = [
        first(D),
        linked(A, DeviceKind::Client, C),
        linked(B, DeviceKind::Client, A),
        linked(C, DeviceKind::Client, B),
    ];
    assert_valid(&certs, D, &first(D));
    for device in [A, B, C] {
        assert_invalid(&certs, device);
    }
}

#[test]
fn an_issuer_that_is_not_a_client_device_is_not_valid() {
    for kind in [DeviceKind::Browser, DeviceKind::Service] {
        let b = linked(B, kind, A);
        let c = linked(C, DeviceKind::Client, B);
        let certs = [first(A), b.clone(), c];
        assert_valid(&certs, B, &b);
        assert_invalid(&certs, C);
    }
    // The master key's own certificate of a non-client device issues nothing either.
    let a = signed(&body(A, DeviceKind::Browser, None), MASTER, A);
    let b = linked(B, DeviceKind::Client, A);
    assert_invalid(&[a, b], B);
}

#[test]
fn an_issuer_of_another_account_is_not_valid() {
    let b = CertBody {
        account: other_account(),
        ..body(B, DeviceKind::Client, Some(A))
    };
    assert_invalid(&[first(A), signed(&b, A, B)], B);
    // Deeper down the chain.
    let a = CertBody {
        account: other_account(),
        ..body(A, DeviceKind::Client, Some(D))
    };
    let certs = [first(D), signed(&a, D, A), linked(B, DeviceKind::Client, A)];
    assert_invalid(&certs, A);
    assert_invalid(&certs, B);
}

#[test]
fn a_chain_with_an_issuer_that_fails_check_1_is_not_valid() {
    let b = linked(B, DeviceKind::Client, A);
    let c = linked(C, DeviceKind::Client, B);
    // No certificate of the issuer.
    assert_invalid(std::slice::from_ref(&b), B);
    assert_invalid(&[first(A), c.clone()], C);
    // The issuer's certificate has a broken proof or signature, or is signed by the server.
    for a in [
        broken_proof(&first(A)),
        broken_signature(&first(A)),
        signed(&body(A, DeviceKind::Client, None), SERVER, A),
    ] {
        let certs = [a, b.clone(), c.clone()];
        assert_invalid(&certs, A);
        assert_invalid(&certs, B);
        assert_invalid(&certs, C);
    }
}

// Copies and bodies.

#[test]
fn copies_of_one_certificate_count_as_one() {
    let a = first(A);
    assert_valid(&[a.clone(), a.clone()], A, &a);
    let b = linked(B, DeviceKind::Client, A);
    assert_valid(&[a.clone(), b.clone(), a.clone(), b.clone()], B, &b);
}

#[test]
fn a_broken_copy_beside_a_genuine_certificate_leaves_the_device_valid() {
    let a = first(A);
    let bytes = a.body_bytes().to_vec();
    let copies = [
        broken_signature(&a),
        broken_proof(&a),
        cert_of(&bytes, &signature(SERVER, CERT, &bytes), a.proof()),
        cert_of(&bytes, &signature(MASTER, PROOF, &bytes), a.proof()),
    ];
    for copy in copies {
        // The check uses the copy whose signature passes check 1, wherever it is.
        assert_valid(&[copy.clone(), a.clone()], A, &a);
        assert_valid(&[a.clone(), copy.clone()], A, &a);
        assert_eq!(
            check(&[copy.clone(), a.clone()], A).unwrap().signature(),
            a.signature()
        );

        // An issuer with a broken copy still issues.
        let b = linked(B, DeviceKind::Client, A);
        assert_valid(&[copy.clone(), b.clone(), a.clone()], B, &b);
    }
}

#[test]
fn a_device_with_two_bodies_has_no_valid_certificate() {
    let a = first(A);
    let renamed = CertBody {
        name: "Another name".to_owned(),
        ..body(A, DeviceKind::Client, None)
    };
    let reissued = linked(A, DeviceKind::Client, B);
    for other in [
        signed(&renamed, MASTER, A),
        // The second body counts once it passes the proof check, whatever its signature.
        signed(&renamed, SERVER, A),
        broken_signature(&signed(&renamed, MASTER, A)),
        reissued,
    ] {
        assert_two_bodies(&[a.clone(), other.clone()], A);
        assert_two_bodies(&[other.clone(), a.clone(), a.clone()], A);
        assert_two_bodies(&[first(B), other, a.clone()], A);
    }
}

#[test]
fn a_second_body_that_fails_the_proof_check_does_not_count() {
    let a = first(A);
    let renamed = CertBody {
        name: "Another name".to_owned(),
        ..body(A, DeviceKind::Client, None)
    };
    let certs = [a.clone(), broken_proof(&signed(&renamed, MASTER, A))];
    assert_valid(&certs, A, &a);
    let certs = [signed(&renamed, MASTER, B), a.clone()];
    assert_valid(&certs, A, &a);
}

#[test]
fn a_device_without_a_certificate_is_not_valid() {
    assert_invalid(&[], A);
    assert_invalid(&[first(A), linked(B, DeviceKind::Client, A)], C);
}
