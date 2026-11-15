//! Revocations the server refuses and does not store: ones that do not decode, and ones that do not
//! count for the session's account.

use serde_json::json;
use skald_proto::{Cbor, DeviceKind, Revocation};

use crate::api::{Session, assert_refused};
use crate::common::device_flow::text;
use crate::common::devices::Key;
use crate::fixture::Fixture;
use crate::kit::{assert_invalid, assert_open, body, by_bobs_laptop, connect, ids, signed, sorted};

/// Bob's laptop revokes bob's phone, both certified on the server; each change of that revocation
/// that keeps it from counting is refused with `invalid-revocation`, and so is the revocation
/// itself from alice's session. Malformed requests and one without a session are refused too.
/// Nothing is stored: no list gives a revocation, and the phone still connects.
#[test]
fn revocations_that_do_not_decode_or_count_are_refused_and_not_stored() {
    let f = Fixture::new(3_131_320_201);
    let db = f.db();
    let t = &f.team;
    let (alice, bob) = (&t.alice, &t.bob);
    let phone = by_bobs_laptop(&f, "Phone");
    let stray = bob.certified_by_key(db, &Key::generate(), "Stray");
    let service = bob.certified_by(db, &bob.laptop, DeviceKind::Service, "Agent");
    let valid = bob.by_device(&bob.laptop, phone.id);

    let mut broken = valid.clone();
    *broken.last_mut().expect("bytes") ^= 1;
    let by_laptop = body(bob, phone.id, Some(bob.laptop.id));
    let bytes = by_laptop.to_cbor();
    let signature = bob.laptop.signing.key.sign("skald cert", &[&bytes]);
    let other_label = Revocation::from_parts(bytes, signature)
        .expect("a revocation")
        .to_cbor();
    let mut also = by_laptop.clone();
    also.also = ids(&[bob.uncertified.id]);
    let also = signed(&also, &bob.laptop.signing.key);
    let by_alice = signed(
        &body(bob, phone.id, Some(alice.laptop.id)),
        &alice.laptop.signing.key,
    );
    let by_alices_master = signed(&body(bob, phone.id, None), &alice.master.key);
    let cases = [
        (valid[..valid.len() - 1].to_vec(), "truncated"),
        (b"not a revocation".to_vec(), "bytes that are no CBOR item"),
        (bob.cert.clone(), "a certificate"),
        (broken, "a broken signature"),
        (other_label, "a signature over another label"),
        (also, "`also` from a device"),
        (by_alice, "a signer of another account"),
        (by_alices_master, "another account's master key"),
        (
            bob.by_device(&bob.uncertified, phone.id),
            "a signer without a certificate",
        ),
        (
            bob.by_device(&stray, phone.id),
            "a signer certified by another master key",
        ),
        (bob.by_device(&service, phone.id), "a `service` signer"),
    ];
    for (revocation, what) in &cases {
        assert_invalid(&bob.session.revoke(revocation), what);
    }
    assert_invalid(
        &alice.session.revoke(&valid),
        "bob's revocation from alice's session",
    );

    for (request, what) in [
        (json!({}), "no revocation"),
        (json!({ "revocation": "not base64url!" }), "not base64url"),
        (json!({ "revocation": 1 }), "not a string"),
    ] {
        let response = bob.session.revoke_body(&request);
        assert_refused(&response, what);
        assert_eq!(response.status, 400, "{what}: {}", text(&response));
    }
    let none = Session::none(&t.flow).revoke(&valid);
    assert_refused(&none, "no session");
    assert_eq!(none.status, 401, "no session: {}", text(&none));

    assert_eq!(bob.session.revocations(), sorted(&[]), "bob's devices");
    assert_eq!(alice.session.revocations(), sorted(&[]), "alice's devices");
    assert_eq!(
        alice.session.space_revocations(t.space),
        sorted(&[]),
        "acme's devices"
    );
    let mut phone = connect(&t.flow.stand.server, &t.flow, bob, &phone);
    assert_open(&mut phone);
}
