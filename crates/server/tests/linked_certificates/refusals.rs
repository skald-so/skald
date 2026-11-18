//! Certificates the server refuses, storing nothing: malformed ones, ones that fail check 1 or 2 of
//! "Device checks" against the account's master key and the certificates it holds, ones that differ
//! from the device's registration, ones for a device the session's account has not registered, and
//! another certificate for a device that holds one. After each, the device's genuine certificate is
//! still taken.

use serde_json::json;
use skald_proto::{Cbor, Cert, CertBody, DeviceKind, PROOF_LABEL};

use crate::accounts::{assert_refused_cert, by_laptop, held, put, register};
use crate::api::Person;
use crate::common::device_flow::{DeviceFlow, INTERVAL, text};
use crate::common::devices::{Device, Key, b64, cert_of, write_devices};
use crate::upload::{issued_by, put_cert_request};

/// Alice and bob on a server of their own.
fn accounts(github: u64) -> (DeviceFlow, Person, Person) {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1"]);
    let alice = Person::new(&flow, github, "alice");
    let bob = Person::new(&flow, github + 1, "bob");
    (flow, alice, bob)
}

/// Checks that alice's desktop's certificate is still taken after the refusals, and is the one the
/// account holds.
#[track_caller]
fn assert_genuine_taken(flow: &DeviceFlow, alice: &Person) {
    let desktop = &alice.uncertified;
    assert_eq!(
        held(alice, desktop),
        None,
        "a refused certificate was stored"
    );
    let genuine = by_laptop(alice, desktop);
    let stored = put(flow, &alice.session, desktop, &genuine);
    assert_eq!(
        stored.status,
        201,
        "the genuine certificate: {}",
        text(&stored)
    );
    assert_eq!(held(alice, desktop), Some(genuine));
}

/// The certificate of the body `bytes` with `signature` and `proof`, encoded.
fn assembled(bytes: &[u8], signature: [u8; 64], proof: [u8; 64]) -> Vec<u8> {
    let cert = Cert::from_parts(bytes.to_vec(), signature, proof);
    cert.expect("a certificate").to_cbor()
}

/// A request that is not `{"cert": <bytes>}`, or bytes that are not a certificate, is refused.
#[test]
fn malformed_requests_and_certificates_are_refused() {
    let (flow, alice, _) = accounts(3_175_200_001);
    let desktop = &alice.uncertified;
    let genuine = by_laptop(&alice, desktop);
    let base = flow.stand.server.base();
    let token = alice.session.token();
    let bodies = [
        ("no certificate", json!({})),
        (
            "a certificate not in base64url",
            json!({ "cert": "not base64url!" }),
        ),
        ("a certificate that is a number", json!({ "cert": 7 })),
        ("an empty certificate", json!({ "cert": "" })),
        (
            "a truncated certificate",
            json!({ "cert": b64(&genuine[..genuine.len() - 1]) }),
        ),
        (
            "a certificate with a byte after it",
            json!({ "cert": b64(&[genuine.clone(), vec![0]].concat()) }),
        ),
    ];
    for (what, body) in &bodies {
        assert_refused_cert(&put_cert_request(base, token, desktop.id, body), what);
    }
    let body = Cert::from_cbor(&genuine)
        .expect("a certificate")
        .body_bytes()
        .to_vec();
    assert_refused_cert(
        &put(&flow, &alice.session, desktop, &body),
        "the body alone",
    );
    assert_genuine_taken(&flow, &alice);
}

/// A certificate whose proof or issuer's signature does not verify is refused.
#[test]
fn certificates_that_fail_the_proof_or_the_issuers_signature_are_refused() {
    let (flow, alice, _) = accounts(3_175_200_011);
    let desktop = &alice.uncertified;
    let mut body = desktop.body_as(alice.account, DeviceKind::Client);
    body.issuer = Some(alice.laptop.id);
    let bytes = body.to_cbor();
    let by_laptop_key = alice
        .laptop
        .signing
        .key
        .sign(skald_proto::CERT_LABEL, &[&bytes]);
    let parts = |signature, proof| assembled(&bytes, signature, proof);
    let stranger = Key::generate();
    let refused: Vec<(&str, Vec<u8>)> = vec![
        (
            "a proof by another key",
            parts(by_laptop_key, stranger.key.sign(PROOF_LABEL, &[&bytes])),
        ),
        (
            "naming the laptop, signed by another key",
            cert_of(&body, &stranger.key, &desktop.signing.key),
        ),
        (
            "naming the laptop, signed by the desktop itself",
            cert_of(&body, &desktop.signing.key, &desktop.signing.key),
        ),
        (
            "naming the laptop, signed by the master key",
            cert_of(&body, &alice.master.key, &desktop.signing.key),
        ),
        (
            "naming the laptop, signed by it over the proof's label",
            parts(
                alice.laptop.signing.key.sign(PROOF_LABEL, &[&bytes]),
                desktop.signing.key.sign(PROOF_LABEL, &[&bytes]),
            ),
        ),
    ];
    for (what, cert) in &refused {
        assert_refused_cert(&put(&flow, &alice.session, desktop, cert), what);
    }
    assert_genuine_taken(&flow, &alice);
}

/// Check 2: the issuer is a `client` device of the account whose chain ends at the account's master
/// key. A `browser` device, a device without a certificate, the new device itself, a device of
/// another account and one certified by another master key are no such issuer.
#[test]
fn certificates_whose_issuer_does_not_chain_to_the_master_key_are_refused() {
    let (flow, alice, bob) = accounts(3_175_200_021);
    let db = &flow.stand.db;
    let desktop = &alice.uncertified;
    let browser = Device::new("Browser");
    let browser_cert = browser.cert_as(alice.account, DeviceKind::Browser, &alice.master);
    write_devices(
        db,
        alice.account,
        DeviceKind::Browser,
        &[(&browser, Some(&browser_cert))],
    );
    let other_master = Key::generate();
    let rogue = Device::new("Rogue");
    let rogue_cert = rogue.cert_as(alice.account, DeviceKind::Client, &other_master);
    write_devices(
        db,
        alice.account,
        DeviceKind::Client,
        &[(&rogue, Some(&rogue_cert))],
    );
    let phone = register(&alice, "Phone");
    let unregistered = Device::new("Nowhere");

    let issued = |issuer: &Device| issued_by(desktop, alice.account, DeviceKind::Client, issuer);
    let refused: Vec<(&str, Vec<u8>)> = vec![
        ("issued by a browser device", issued(&browser)),
        ("issued by a device without a certificate", issued(&phone)),
        ("issued by the new device itself", issued(desktop)),
        (
            "issued by a device the account never registered",
            issued(&unregistered),
        ),
        ("issued by another account's laptop", issued(&bob.laptop)),
        (
            "issued by a device certified by another master key",
            issued(&rogue),
        ),
    ];
    for (what, cert) in &refused {
        assert_refused_cert(&put(&flow, &alice.session, desktop, cert), what);
    }
    assert_eq!(held(&alice, &phone), None);
    assert_genuine_taken(&flow, &alice);
}

/// The certificate's account, device, kind, keys and `name` must be those the device registered on
/// the session's account.
#[test]
fn certificates_that_differ_from_the_registration_are_refused() {
    let (flow, alice, bob) = accounts(3_175_200_031);
    let desktop = &alice.uncertified;
    let changed = |change: &dyn Fn(&mut CertBody)| {
        let mut body = desktop.body_as(alice.account, DeviceKind::Client);
        body.issuer = Some(alice.laptop.id);
        change(&mut body);
        cert_of(&body, &alice.laptop.signing.key, &desktop.signing.key)
    };
    let other_keys = Device::new(&desktop.name);
    let refused: Vec<(&str, Vec<u8>)> = vec![
        (
            "another account",
            changed(&|body| body.account = bob.account),
        ),
        ("another hpke key", changed(&|body| body.hpke = [9; 32])),
        (
            "another name",
            changed(&|body| body.name = "Phone".to_owned()),
        ),
        (
            "another kind",
            changed(&|body| body.kind = DeviceKind::Browser),
        ),
        (
            "a service device",
            changed(&|body| body.kind = DeviceKind::Service),
        ),
        (
            "other keys under the desktop's ID",
            cert_of(
                &CertBody {
                    device: desktop.id,
                    ..other_keys.body_as(alice.account, DeviceKind::Client)
                },
                &alice.laptop.signing.key,
                &other_keys.signing.key,
            ),
        ),
    ];
    for (what, cert) in &refused {
        assert_refused_cert(&put(&flow, &alice.session, desktop, cert), what);
    }
    assert_genuine_taken(&flow, &alice);
}

/// A session stores certificates only of devices its account registered: not one the server does
/// not hold, nor another account's, also when issued for that account.
#[test]
fn certificates_of_devices_the_account_has_not_registered_are_refused() {
    let (flow, alice, bob) = accounts(3_175_200_041);
    let unregistered = Device::new("Nowhere");
    assert_refused_cert(
        &put(
            &flow,
            &alice.session,
            &unregistered,
            &by_laptop(&alice, &unregistered),
        ),
        "a device the server does not hold",
    );
    // Bob's desktop, issued on alice's account by alice's laptop, and on bob's by bob's.
    let bobs = &bob.uncertified;
    assert_refused_cert(
        &put(&flow, &alice.session, bobs, &by_laptop(&alice, bobs)),
        "bob's desktop on alice's account, from alice's session",
    );
    assert_refused_cert(
        &put(&flow, &alice.session, bobs, &by_laptop(&bob, bobs)),
        "bob's desktop, issued by bob's laptop, from alice's session",
    );
    // Alice's genuine certificate, from bob's session.
    let desktop = &alice.uncertified;
    assert_refused_cert(
        &put(&flow, &bob.session, desktop, &by_laptop(&alice, desktop)),
        "alice's desktop from bob's session",
    );
    assert_eq!(held(&bob, bobs), None, "bob's desktop");
    let stored = put(&flow, &bob.session, bobs, &by_laptop(&bob, bobs));
    assert_eq!(stored.status, 201, "bob's own: {}", text(&stored));
    assert_genuine_taken(&flow, &alice);
}

/// A device holds one certificate: another one for it, valid as it is, is refused and the one it
/// holds stays, also for the first device, which the master key certified.
#[test]
fn another_certificate_of_a_device_that_holds_one_is_refused() {
    let (flow, alice, _) = accounts(3_175_200_051);
    assert_genuine_taken(&flow, &alice);
    let desktop = &alice.uncertified;
    let genuine = held(&alice, desktop);

    let mut later = desktop.body_as(alice.account, DeviceKind::Client);
    later.issuer = Some(alice.laptop.id);
    later.issued += 60;
    let other = cert_of(&later, &alice.laptop.signing.key, &desktop.signing.key);
    let response = put(&flow, &alice.session, desktop, &other);
    assert_refused_cert(&response, "another certificate for the desktop");
    assert_eq!(
        held(&alice, desktop),
        genuine,
        "the desktop's certificate stays"
    );

    let laptop = &alice.laptop;
    let by_desktop = issued_by(laptop, alice.account, DeviceKind::Client, desktop);
    assert_refused_cert(
        &put(&flow, &alice.session, laptop, &by_desktop),
        "a certificate of the first device by the desktop",
    );
    assert_eq!(held(&alice, laptop), Some(alice.cert.clone()));

    // Without a session.
    let none = put_cert_request(
        flow.stand.server.base(),
        "unknown-token",
        desktop.id,
        &json!({ "cert": b64(&other) }),
    );
    assert_eq!(none.status, 401, "an unknown session: {}", text(&none));
}
