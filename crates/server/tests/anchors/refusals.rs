//! Anchors objects the server refuses and does not store: ones that do not decode, ones of another
//! account than the session's, and ones whose device is no `client` device of the account with a
//! valid certificate or whose signature does not verify with it.

use serde_json::json;
use skald_proto::DeviceKind;

use crate::api::{Session, assert_refused};
use crate::common::cbor::{array_of, byte_item, uint};
use crate::common::device_flow::text;
use crate::common::devices::{Device, Key};
use crate::fixture::Fixture;
use crate::kit::sorted;
use crate::objects::{
    ACME, ANCHORS, PERSONAL, assert_invalid, assert_stored, body, map, signed, signed_as,
};

/// Bob's laptop's anchors object is refused with `invalid-anchors` from alice's session, and so is
/// each change of it that keeps it from passing: malformed bytes, a broken signature or one over
/// another label or by another key, another account, and a device of another account, without a
/// certificate, unknown to the server, certified by another master key or of another kind than
/// `client`. Malformed requests and ones without a session are refused too. Nothing is stored: no
/// session gets an anchors object, and the laptop's is then stored as new.
#[test]
fn anchors_objects_that_do_not_decode_or_pass_are_refused_and_not_stored() {
    let f = Fixture::new(3_172_020_301);
    let db = f.db();
    let t = &f.team;
    let (alice, bob) = (&t.alice, &t.bob);
    let laptop = &bob.laptop;
    let valid = bob.anchors_of(laptop, 1);
    let stray = bob.certified_by_key(db, &Key::generate(), "Stray");
    let browser = bob.certified_by(db, laptop, DeviceKind::Browser, "Browser");
    let service = bob.certified_by(db, laptop, DeviceKind::Service, "Agent");
    let stranger = Device::new("Stranger");

    let mut broken = valid.clone();
    *broken.last_mut().expect("bytes") ^= 1;
    let laptops = bob.anchors_body(laptop, 1);
    let of_device = |device: &Device| bob.anchors_of(device, 1);
    let revision = [0xa1; 32];
    let malformed = |fields: &[(u64, Vec<u8>)]| signed(&map(fields), &laptop.signing.key);
    let cases = [
        (valid[..valid.len() - 1].to_vec(), "truncated"),
        (b"not anchors".to_vec(), "bytes that are no CBOR item"),
        (bob.cert.clone(), "a certificate"),
        (broken, "a broken signature"),
        (
            signed_as(&laptops, &laptop.signing.key, "skald revocation"),
            "a signature over another label",
        ),
        (
            signed(&laptops, &alice.laptop.signing.key),
            "a signature by another device's key",
        ),
        (
            signed(
                &body(alice.account, laptop.id, &[(ACME, &revision)], None),
                &laptop.signing.key,
            ),
            "another account",
        ),
        (
            signed(
                &body(bob.account, alice.laptop.id, &[(ACME, &revision)], None),
                &alice.laptop.signing.key,
            ),
            "a device of another account",
        ),
        (
            of_device(&bob.uncertified),
            "a device without a certificate",
        ),
        (of_device(&stranger), "a device the server does not know"),
        (
            of_device(&stray),
            "a device certified by another master key",
        ),
        (of_device(&browser), "a `browser` device"),
        (of_device(&service), "a `service` device"),
        (
            malformed(&[
                (1, byte_item(bob.account.as_bytes())),
                (2, byte_item(laptop.id.as_bytes())),
            ]),
            "a body without anchors",
        ),
        (
            signed(
                &body(
                    bob.account,
                    laptop.id,
                    &[(ACME, &[0xa1; 31])],
                    Some(PERSONAL),
                ),
                &laptop.signing.key,
            ),
            "a revision hash of 31 bytes",
        ),
        (
            malformed(&[
                (1, byte_item(bob.account.as_bytes())),
                (2, byte_item(laptop.id.as_bytes())),
                (3, array_of(&[])),
                (4, uint(4)),
            ]),
            "a personal space that is a number",
        ),
    ];
    for (anchors, what) in &cases {
        assert_invalid(&bob.session.store_anchors(anchors), what);
    }
    assert_invalid(
        &alice.session.store_anchors(&valid),
        "bob's anchors object from alice's session",
    );

    for (request, what) in [
        (json!({}), "no anchors"),
        (json!({ "anchors": "not base64url!" }), "not base64url"),
        (json!({ "anchors": 1 }), "not a string"),
    ] {
        let response = bob.session.store_anchors_body(&request);
        assert_refused(&response, what);
        assert_eq!(response.status, 400, "{what}: {}", text(&response));
    }
    let none = Session::none(&t.flow);
    let response = none.store_anchors(&valid);
    assert_eq!(response.status, 401, "no session: {}", text(&response));
    let response = none.call("get", ANCHORS, None);
    assert_eq!(response.status, 401, "no session: {}", text(&response));

    assert_eq!(bob.session.anchors(), sorted(&[]), "bob's: nothing stored");
    assert_eq!(
        alice.session.anchors(),
        sorted(&[]),
        "alice's: nothing stored"
    );
    assert_stored(&bob.session, &valid, "the laptop's, after the refusals");
}
