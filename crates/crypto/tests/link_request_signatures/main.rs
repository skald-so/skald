//! A link code's request with the new device's SSH signature and its GitHub user, natively and in
//! WASM (`docs/protocol/membership.md`, "Link code"): beside its kind, keys and `name`, a
//! `link-request-body` carries the `ssh` and `github` the new device made for its certificate when
//! it has them, as a certificate does ("Device certificate"). Requests with them and without them
//! keep their bytes, fields of later versions included.
//!
//! What the tests chose, beside what the spec names: the request carries them as `5 => bstr`, the
//! SSH signature, and `6 => uint`, the GitHub user ID, and `skald_proto::LinkRequestBody` has them
//! as `ssh: Option<Vec<u8>>` and `github: Option<u64>`, as `CertBody` does.

// The keys and devices of the tests of revocations, the requests of the tests of link objects, the
// fixed SSH signatures of the tests of SSH signatures and the shared CBOR and field helpers; this
// target uses some of each.
#![allow(dead_code)]

#[path = "../rosters/cbor.rs"]
mod cbor;
#[path = "../common/mod.rs"]
mod common;
#[path = "../common/fields.rs"]
mod fields;
#[path = "../revocations/fixtures.rs"]
mod fixtures;
#[path = "../link_objects/objects.rs"]
mod objects;
#[path = "../ssh_signatures/fixtures.rs"]
mod ssh;

use skald_proto::{Cbor, DeviceKind, LinkRequestBody, Unknown};

use cbor::{Value, bytes, text};
use fields::{truncated_and_longer, unknown, with};
use fixtures::{DESKTOP, public};
use objects::request_fields;

/// The request's fields of the SSH signature and of its GitHub user.
const SSH_FIELD: i64 = 5;
const GITHUB_FIELD: i64 = 6;

/// The fields of a request with the SSH signature `ssh` by a key of the GitHub user `github`.
fn signed_request_fields(ssh: &[u8], github: u64) -> Vec<(i64, Value)> {
    let mut fields = request_fields();
    fields.push((SSH_FIELD, bytes(ssh)));
    fields.push((GITHUB_FIELD, Value::Uint(github)));
    fields
}

/// Fails unless the encoded request `encoded` decodes into `expected` and encodes back byte for
/// byte.
#[track_caller]
fn assert_round_trip(encoded: &[u8], expected: &LinkRequestBody) {
    let decoded = LinkRequestBody::from_cbor(encoded).expect("a link request body");
    assert_eq!(&decoded, expected);
    assert_eq!(decoded.to_cbor(), encoded, "byte for byte");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_request_with_an_ssh_signature_and_its_github_user_round_trips_byte_for_byte() {
    let sig = ssh::ed25519_sig();
    let built = LinkRequestBody {
        kind: DeviceKind::Client,
        signing: public(DESKTOP),
        hpke: [0x3d; 32],
        name: "Desktop at the office".to_owned(),
        ssh: Some(sig.clone()),
        github: Some(ssh::GITHUB),
        unknown: Unknown::default(),
    };
    let encoded = Value::Map(signed_request_fields(&sig, ssh::GITHUB)).encode();
    assert_eq!(
        built.to_cbor(),
        encoded,
        "the request encodes `ssh` at 5 and `github` at 6, in the deterministic encoding"
    );
    assert_round_trip(&encoded, &built);

    let mut later = signed_request_fields(&sig, ssh::GITHUB);
    later.extend(unknown());
    let encoded = Value::Map(later).encode();
    let decoded = LinkRequestBody::from_cbor(&encoded).expect("a request with later fields");
    assert_eq!(decoded.ssh.as_deref(), Some(&sig[..]));
    assert_eq!(decoded.github, Some(ssh::GITHUB));
    let mut keys: Vec<i128> = decoded.unknown.keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, [-3, 24]);
    assert_round_trip(&encoded, &decoded);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_request_without_them_round_trips_byte_for_byte() {
    for extra in [vec![], unknown()] {
        let mut fields = request_fields();
        fields.extend(extra);
        let encoded = Value::Map(fields).encode();
        let decoded = LinkRequestBody::from_cbor(&encoded).expect("a link request body");
        assert_eq!(decoded.ssh, None, "an SSH signature");
        assert_eq!(decoded.github, None, "a GitHub user");
        assert_round_trip(&encoded, &decoded);
    }
    let built = LinkRequestBody {
        kind: DeviceKind::Client,
        signing: public(DESKTOP),
        hpke: [0x3d; 32],
        name: "Desktop at the office".to_owned(),
        ssh: None,
        github: None,
        unknown: Unknown::default(),
    };
    assert_eq!(
        built.to_cbor(),
        Value::Map(request_fields()).encode(),
        "a request without them encodes neither"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_request_whose_ssh_signature_or_github_user_is_of_another_type_is_refused() {
    let fields = signed_request_fields(&ssh::ed25519_sig(), ssh::GITHUB);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "an SSH signature that is text",
            with(&fields, SSH_FIELD, text("U1NIU0lH")),
        ),
        (
            "a GitHub user that is text",
            with(&fields, GITHUB_FIELD, text("583231")),
        ),
        (
            "a GitHub user that is bytes",
            with(&fields, GITHUB_FIELD, bytes(&ssh::GITHUB.to_be_bytes())),
        ),
    ];
    for (what, encoded) in &cases {
        assert!(
            LinkRequestBody::from_cbor(encoded).is_err(),
            "a request with {what}"
        );
    }
    for input in truncated_and_longer(&Value::Map(fields).encode()) {
        assert!(LinkRequestBody::from_cbor(&input).is_err(), "{input:?}");
    }
}
