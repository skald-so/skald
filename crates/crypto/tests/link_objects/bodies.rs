//! The request and grant bodies of a link code's run (`membership.md`, "Link code"): a
//! `link-request-body` names the new device's kind, keys and name; a `link-grant-body` carries the
//! master key, the new device's certificate body as it was signed, the issuing device's signature
//! over it and the link object. Both keep fields of later versions and their bytes; malformed ones
//! are refused.

use skald_proto::{Cbor, CertBody, DeviceKind, LinkGrantBody, LinkRequestBody, Unknown};

use crate::cbor::{Value, bytes, text};
use crate::fields::{truncated_and_longer, unknown, with, without};
use crate::fixtures::{ACCOUNT, DESKTOP, LAPTOP, MASTER, device, public, sign};
use crate::objects::{CERT, Linked, cert_body_fields, grant_fields, request_fields};

/// The fields with `later` ones after them.
fn and_later(mut fields: Vec<(i64, Value)>, later: &[(i64, Value)]) -> Vec<(i64, Value)> {
    fields.extend(later.iter().cloned());
    fields
}

/// The encoded certificate body of the grant.
fn cert() -> Vec<u8> {
    Value::Map(cert_body_fields()).encode()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_request_body_decodes_into_its_spec_fields_and_encodes_back() {
    let encoded = Value::Map(request_fields()).encode();
    let request = LinkRequestBody::from_cbor(&encoded).expect("a link request body");
    assert_eq!(request.kind, DeviceKind::Client);
    assert_eq!(request.signing, public(DESKTOP));
    assert_eq!(request.hpke, [0x3d; 32]);
    assert_eq!(request.name, "Desktop at the office");
    assert!(request.unknown.is_empty());
    assert_eq!(request.to_cbor(), encoded, "byte for byte");

    let built = LinkRequestBody {
        kind: DeviceKind::Client,
        signing: public(DESKTOP),
        hpke: [0x3d; 32],
        name: "Desktop at the office".to_owned(),
        unknown: Unknown::default(),
    };
    assert_eq!(
        built.to_cbor(),
        encoded,
        "a built request, in the deterministic encoding"
    );

    let later = Value::Map(and_later(request_fields(), &unknown())).encode();
    let request = LinkRequestBody::from_cbor(&later).expect("a request with later fields");
    assert_eq!(request.unknown.keys().collect::<Vec<_>>(), [-3, 24]);
    assert_eq!(request.to_cbor(), later, "with later fields, byte for byte");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_link_request_bodies_are_refused() {
    let fields = request_fields();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("without a kind", without(&fields, 1)),
        ("without a signing key", without(&fields, 2)),
        ("without an HPKE key", without(&fields, 3)),
        ("without a name", without(&fields, 4)),
        ("a kind that is text", with(&fields, 1, text("client"))),
        (
            "a signing key of 31 bytes",
            with(&fields, 2, bytes(&[0x5c; 31])),
        ),
        (
            "an HPKE key of 33 bytes",
            with(&fields, 3, bytes(&[0x3d; 33])),
        ),
        ("an empty name", with(&fields, 4, text(""))),
        (
            "a name of 65 bytes",
            with(&fields, 4, text(&"n".repeat(65))),
        ),
        ("a name that is bytes", with(&fields, 4, bytes(b"Desktop"))),
    ];
    for (what, encoded) in &cases {
        assert!(
            LinkRequestBody::from_cbor(encoded).is_err(),
            "a request {what}"
        );
    }
    for input in truncated_and_longer(&Value::Map(fields).encode()) {
        assert!(LinkRequestBody::from_cbor(&input).is_err(), "{input:?}");
    }
    let longest = with(&request_fields(), 4, text(&"n".repeat(64)));
    assert!(
        LinkRequestBody::from_cbor(&longest).is_ok(),
        "a name of 64 bytes is a name"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_link_grant_body_decodes_into_its_spec_fields_and_encodes_back() {
    let linked = Linked::desktop();
    let encoded = Value::Map(grant_fields(&cert(), &linked, &[])).encode();
    let grant = LinkGrantBody::from_cbor(&encoded).expect("a link grant body");
    assert_eq!(grant.master, public(MASTER));
    assert_eq!(grant.cert, cert(), "the certificate body as it was signed");
    assert_eq!(grant.signature, sign(LAPTOP, CERT, &cert()));
    assert_eq!(grant.link.to_cbor(), linked.encoded(), "the link object");
    assert_eq!(grant.link.body().device, device(DESKTOP));
    assert!(grant.unknown.is_empty());
    assert_eq!(grant.to_cbor(), encoded, "byte for byte");

    let body = CertBody::from_cbor(&grant.cert).expect("the grant's certificate body");
    assert_eq!(body.account, ACCOUNT);
    assert_eq!(body.device, device(DESKTOP));
    assert_eq!(body.issuer, Some(device(LAPTOP)));
    assert_eq!(body.name, "Desktop at the office");

    let later_link = Linked {
        later: unknown(),
        ..linked
    };
    let later = Value::Map(grant_fields(&cert(), &later_link, &unknown())).encode();
    let grant = LinkGrantBody::from_cbor(&later).expect("a grant with later fields");
    assert_eq!(grant.unknown.keys().collect::<Vec<_>>(), [-3, 24]);
    assert_eq!(grant.link.to_cbor(), later_link.encoded());
    assert_eq!(grant.to_cbor(), later, "with later fields, byte for byte");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_grant_keeps_the_bytes_of_a_certificate_body_with_later_fields() {
    let cert = Value::Map(and_later(cert_body_fields(), &unknown())).encode();
    let encoded = Value::Map(grant_fields(&cert, &Linked::desktop(), &[])).encode();
    let grant = LinkGrantBody::from_cbor(&encoded).expect("a grant");
    assert_eq!(grant.cert, cert, "the certificate body, byte for byte");
    assert_eq!(grant.to_cbor(), encoded);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_link_grant_bodies_are_refused() {
    let linked = Linked::desktop();
    let fields = grant_fields(&cert(), &linked, &[]);
    let not_a_cert = without(&cert_body_fields(), 9);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("without a master key", without(&fields, 1)),
        ("without a certificate", without(&fields, 2)),
        ("without a signature", without(&fields, 3)),
        ("without a link object", without(&fields, 4)),
        (
            "a master key of 31 bytes",
            with(&fields, 1, bytes(&[0x01; 31])),
        ),
        (
            "a signature of 32 bytes",
            with(&fields, 3, bytes(&[0x5a; 32])),
        ),
        (
            "a certificate body without a name",
            with(&fields, 2, bytes(&not_a_cert)),
        ),
        (
            "a certificate that is not CBOR",
            with(&fields, 2, bytes(&[0xff, 0x00])),
        ),
        (
            "a certificate as a map in place of its bytes",
            with(&fields, 2, Value::Map(cert_body_fields())),
        ),
        (
            "a link object as its bytes",
            with(&fields, 4, bytes(&linked.encoded())),
        ),
        (
            "a link object of one item",
            with(&fields, 4, Value::Array(vec![bytes(&linked.body())])),
        ),
    ];
    for (what, encoded) in &cases {
        assert!(LinkGrantBody::from_cbor(encoded).is_err(), "a grant {what}");
    }
    for input in truncated_and_longer(&Value::Map(fields).encode()) {
        assert!(LinkGrantBody::from_cbor(&input).is_err(), "{input:?}");
    }
}
