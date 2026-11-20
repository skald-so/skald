//! The sealed presence value of `docs/protocol/test-vectors.md`, section "Presence value": its
//! history key, its device's signing key (an Ed25519 secret key), its nonce and its plaintext, the
//! encoded `presence-plaintext` before padding, each as a list item `- <name>: `<hex>``, and the
//! sealed value in its `hex` block. It is genuine: it opens as the spec says apart from the crate,
//! `skald-crypto` opens it, and sealing its body with its keys gives its plaintext.

use skald_crypto::SigningKey;
use skald_proto::{Cbor, DeviceId, Presence, PresenceBody, Signed};

use crate::common::hex;
use crate::fixtures::{BLOCK, LABEL, aad, decrypt, derived, padded};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");
const SECTION: &str = "## Presence value";

/// The lines of the section, up to the next section.
fn section() -> Vec<&'static str> {
    let start = DOC
        .lines()
        .position(|line| line == SECTION)
        .unwrap_or_else(|| panic!("test-vectors.md has no section {SECTION:?}"));
    DOC.lines()
        .skip(start + 1)
        .take_while(|line| !line.starts_with("## "))
        .collect()
}

/// The hex of the list item `- <name>: `<hex>`` of the section.
fn item(name: &str) -> Vec<u8> {
    let prefix = format!("- {name}: ");
    let line = section()
        .into_iter()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("{SECTION:?} has no item {name:?}"));
    hex(line.trim().trim_matches('`'))
}

/// The bytes of the section's `hex` block: the sealed value.
fn value_bytes() -> Vec<u8> {
    let lines = section();
    let mut block = lines
        .iter()
        .skip_while(|line| **line != "```hex")
        .skip(1)
        .peekable();
    assert!(block.peek().is_some(), "{SECTION:?} has no hex block");
    let digits: String = block.take_while(|line| **line != "```").copied().collect();
    hex(&digits)
}

struct Vector {
    history: [u8; 16],
    signing: SigningKey,
    nonce: [u8; 12],
    plaintext: Vec<u8>,
    encoded: Vec<u8>,
    value: Presence,
}

fn vector() -> Vector {
    let encoded = value_bytes();
    let value = Presence::from_cbor(&encoded).expect("the published value decodes");
    Vector {
        history: item("History key")
            .try_into()
            .expect("a 16-byte history key"),
        signing: SigningKey::from_bytes(
            &item("Signing key")
                .try_into()
                .expect("a 32-byte signing key"),
        ),
        nonce: item("Nonce").try_into().expect("a 12-byte nonce"),
        plaintext: item("Plaintext"),
        encoded,
        value,
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_presence_value_that_opens_as_the_spec_says() {
    let vector = vector();
    let value = &vector.value;
    assert_eq!(value.to_cbor(), vector.encoded, "the value, byte for byte");
    assert_eq!(
        value.nonce, vector.nonce,
        "the value's nonce is the published one"
    );
    assert_eq!(
        value.device,
        DeviceId::from_signing_key(&vector.signing.public_key()),
        "the value's device is that of the signing key"
    );

    let opened = decrypt(
        &derived(&vector.history),
        &value.nonce,
        &aad(value.device, value.channel, value.epoch),
        &value.ct,
    )
    .expect("the value opens with the presence key of its history key");
    assert_eq!(opened.len() % BLOCK, 0);
    assert_eq!(
        opened,
        padded(&vector.plaintext),
        "the plaintext, padded with zeros"
    );

    let plaintext = Signed::<PresenceBody>::from_cbor(&vector.plaintext)
        .expect("the published plaintext decodes");
    assert_eq!(plaintext.to_cbor(), vector.plaintext, "byte for byte");
    assert_eq!(
        plaintext.body().to_cbor(),
        plaintext.body_bytes(),
        "the body, byte for byte"
    );
    skald_crypto::verify(
        &vector.signing.public_key(),
        LABEL,
        &[plaintext.body_bytes()],
        plaintext.signature(),
    )
    .expect("signed by the signing key over [\"skald presence\", body]");

    let body = plaintext.body();
    assert_eq!(
        (body.device, body.channel, body.epoch),
        (value.device, value.channel, value.epoch)
    );
    assert!(
        body.agents
            .iter()
            .any(|agent| agent.note.is_some() && agent.runtime.is_some()),
        "an agent with a note and a runtime"
    );
    assert!(!body.serves.is_empty(), "a served queue");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_crate_opens_the_presence_vector_and_seals_its_body_to_its_plaintext() {
    let vector = vector();
    let body = Signed::<PresenceBody>::from_cbor(&vector.plaintext)
        .expect("the published plaintext decodes")
        .body()
        .clone();
    let key = skald_crypto::HistoryKey::from_bytes(&vector.history).presence_key();
    let public = vector.signing.public_key();

    let opened = key
        .open(&vector.value, &public, body.expires - 60)
        .expect("the crate opens the published value before its expires");
    assert_eq!(opened, body);

    let sealed = key
        .seal(&body, &vector.signing)
        .expect("the crate seals the body");
    assert_eq!(
        (sealed.device, sealed.channel, sealed.epoch),
        (
            vector.value.device,
            vector.value.channel,
            vector.value.epoch
        )
    );
    let plaintext = decrypt(
        &derived(&vector.history),
        &sealed.nonce,
        &aad(sealed.device, sealed.channel, sealed.epoch),
        &sealed.ct,
    )
    .expect("the crate's value opens as the spec says");
    assert_eq!(
        plaintext,
        padded(&vector.plaintext),
        "the crate's plaintext is the published one"
    );
    assert_eq!(key.open(&sealed, &public, body.expires - 60), Ok(body));
}
