//! `docs/protocol/test-vectors.md`, "Certificate with an SSH signature": a `client` certificate
//! with `ssh` and `github` that decodes and encodes back to the published bytes, its `ssh` an
//! SSH signature by an Ed25519 key that verifies with that key.

use skald_crypto::{SshCheck, check_ssh_signature};
use skald_proto::{Cbor, Cert, CertBody, DeviceKind};

use crate::common::hex;
use crate::fixtures::signer_line;

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");
const SECTION: &str = "Certificate with an SSH signature";

/// The first `hex` block of the section [`SECTION`].
fn vector() -> Vec<u8> {
    let doc = DOC.replace("\r\n", "\n");
    let heading = format!("\n## {SECTION}\n");
    let (_, rest) = doc
        .split_once(&heading)
        .unwrap_or_else(|| panic!("test-vectors.md has no section {SECTION:?}"));
    let section = rest.split("\n## ").next().unwrap();
    let (_, block) = section
        .split_once("```hex\n")
        .unwrap_or_else(|| panic!("{SECTION:?} has no hex block"));
    hex(block.split("```").next().unwrap())
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_certificate_with_an_ssh_signature() {
    let encoded = vector();
    let cert = Cert::from_cbor(&encoded).expect("the published certificate decodes");
    assert_eq!(cert.to_cbor(), encoded, "the certificate, byte for byte");
    let body = CertBody::from_cbor(cert.body_bytes()).unwrap();
    assert_eq!(body.to_cbor(), cert.body_bytes(), "the body, byte for byte");
    assert_eq!(body.kind, DeviceKind::Client);
    assert!(body.github.is_some(), "it names a GitHub user");

    let sig = body.ssh.clone().expect("it carries an SSH signature");
    let signer = signer_line(&sig);
    assert!(
        signer.starts_with("ssh-ed25519 "),
        "an Ed25519 key signs it: {signer}"
    );
    assert_eq!(
        check_ssh_signature(&body, &[signer.as_str()]),
        SshCheck::Passed { key: 0 }
    );
}
