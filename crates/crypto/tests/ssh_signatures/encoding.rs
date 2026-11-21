//! A certificate carries `ssh` and `github`, the GitHub user ID, both or neither: one with only one
//! of them is malformed. Only `client` certificates carry them, and a client ignores them on others,
//! so other kinds with them still decode. Certificates keep their bytes, unknown fields included.

use skald_proto::{Cbor, Cert, CertBody};

use crate::cbor::{Value, bytes, text};
use crate::fixtures::{
    BROWSER, CLIENT, GITHUB, GITHUB_FIELD, SERVICE, SSH_FIELD, body, cert_bytes, ed25519_sig,
    encoded, fields, replaced, signed_fields,
};

/// Map fields of a later version, which decoding keeps and encoding gives back.
fn unknown() -> Vec<(i64, Value)> {
    vec![(-3, text("later")), (24, Value::Bool(true))]
}

/// Checks that the encoded body `body` decodes into `expected` and encodes back byte for byte, on
/// its own and in a certificate.
fn assert_round_trip(body: &[u8], expected: &CertBody) {
    let decoded = CertBody::from_cbor(body).expect("the certificate body decodes");
    assert_eq!(&decoded, expected);
    assert_eq!(decoded.to_cbor(), body, "the body, byte for byte");
    let encoded = cert_bytes(body);
    let cert = Cert::from_cbor(&encoded).expect("the certificate decodes");
    assert_eq!(cert.body(), expected);
    assert_eq!(cert.body_bytes(), body);
    assert_eq!(cert.to_cbor(), encoded, "the certificate, byte for byte");
}

/// Checks that the encoded body `body` is refused, on its own and in a certificate.
fn assert_refused(body: &[u8], what: &str) {
    assert!(CertBody::from_cbor(body).is_err(), "{what}: the body");
    assert!(
        Cert::from_cbor(&cert_bytes(body)).is_err(),
        "{what}: the certificate"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_certificate_with_an_ssh_signature_and_its_github_user_round_trips_byte_for_byte() {
    let sig = ed25519_sig();
    let without = body(fields(CLIENT));
    let with = CertBody {
        ssh: Some(sig.clone()),
        github: Some(GITHUB),
        ..without
    };
    assert_eq!(
        with.to_cbor(),
        encoded(signed_fields(CLIENT, &sig)),
        "the body encodes `ssh` at 8 and `github` at 10"
    );
    assert_round_trip(&encoded(signed_fields(CLIENT, &sig)), &with);

    let mut fields = signed_fields(CLIENT, &sig);
    fields.extend(unknown());
    let decoded = body(fields.clone());
    assert_eq!(decoded.ssh.as_deref(), Some(&sig[..]));
    assert_eq!(decoded.github, Some(GITHUB));
    assert_round_trip(&encoded(fields), &decoded);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_certificate_with_neither_round_trips_byte_for_byte() {
    for extra in [vec![], unknown()] {
        let mut fields = fields(CLIENT);
        fields.extend(extra);
        let decoded = body(fields.clone());
        assert_eq!(decoded.ssh, None);
        assert_eq!(decoded.github, None);
        assert_round_trip(&encoded(fields), &decoded);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_certificate_with_only_an_ssh_signature_is_refused() {
    for extra in [vec![], unknown()] {
        let mut fields = fields(CLIENT);
        fields.extend(extra);
        fields.push((SSH_FIELD, bytes(&ed25519_sig())));
        assert_refused(&encoded(fields), "only `ssh`");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_certificate_with_only_a_github_user_is_refused() {
    for extra in [vec![], unknown()] {
        let mut fields = fields(CLIENT);
        fields.extend(extra);
        fields.push((GITHUB_FIELD, Value::Uint(GITHUB)));
        assert_refused(&encoded(fields), "only `github`");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_github_user_that_is_not_an_unsigned_integer_is_refused() {
    for value in [text("583231"), bytes(&GITHUB.to_be_bytes())] {
        let fields = replaced(signed_fields(CLIENT, &ed25519_sig()), GITHUB_FIELD, value);
        assert_refused(&encoded(fields), "a `github` of another type");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn browser_and_service_certificates_with_both_still_decode() {
    for kind in [BROWSER, SERVICE] {
        let fields = signed_fields(kind, &ed25519_sig());
        let decoded = body(fields.clone());
        assert_round_trip(&encoded(fields), &decoded);
    }
}
