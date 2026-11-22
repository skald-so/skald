//! Sealing and opening presence values: the presence key of an epoch is HKDF-SHA256 of its history
//! key; a value holds its body, signed by its device over `["skald presence", body]` and padded with
//! zeros to a multiple of 4096 bytes, sealed with AES-128-GCM under a random nonce and the
//! associated data `["skald presence", device, channel, epoch]`. A value opens with the epoch's
//! history key to the body, unknown fields included.

use skald_proto::{Cbor, ChannelId, Presence};

use crate::devices::device_id;
use crate::fields::unknown;
use crate::fixtures::*;

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_sealed_value_is_the_signed_body_padded_and_sealed_under_the_presence_key_as_the_spec_says() {
    let body = Body::of(ALICE, ChannelId::generate());
    let value = seal(&HISTORY, &body.decoded(), ALICE);
    assert_eq!(
        (value.device, value.channel, value.epoch),
        (device_id(ALICE), body.channel, 1),
        "the value's fields are the body's"
    );

    let plaintext = decrypted(&HISTORY, &value)
        .expect("the value opens with the presence key and the spec's associated data");
    assert_eq!(value.ct.len(), BLOCK + 16, "one block and the tag");
    assert_eq!(
        plaintext,
        padded(&body.signed(ALICE, LABEL)),
        "the body signed by its device over [\"skald presence\", body], padded with zeros"
    );
    assert!(
        decrypt(
            &HISTORY,
            &value.nonce,
            &aad(value.device, value.channel, value.epoch),
            &value.ct
        )
        .is_none(),
        "the presence key is derived, not the history key itself"
    );

    let again = seal(&HISTORY, &body.decoded(), ALICE);
    assert_ne!(again.nonce, value.nonce, "a fresh random nonce each time");
    assert_ne!(again.ct, value.ct);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_over_one_block_is_padded_to_the_next_multiple_of_4096_bytes() {
    let mut body = Body::of(ALICE, ChannelId::generate());
    body.agents = (0..40)
        .map(|n| {
            status_with(
                &format!("acme/alice/agent-{n}@skald.example"),
                AVAILABLE,
                &"w".repeat(100),
                "claude-code",
            )
        })
        .collect();
    let signed = body.signed(ALICE, LABEL);
    assert!(signed.len() > BLOCK && signed.len() < 2 * BLOCK);

    let value = seal(&HISTORY, &body.decoded(), ALICE);
    assert_eq!(value.ct.len(), 2 * BLOCK + 16, "two blocks and the tag");
    assert_eq!(decrypted(&HISTORY, &value), Some(padded(&signed)));
    assert_eq!(open(&value, ALICE), Ok(body.decoded()));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_sealed_value_round_trips_byte_for_byte_and_opens_to_its_body_with_unknown_fields() {
    let mut body = Body::of(BOB, ChannelId::generate());
    body.epoch = 4;
    body.agents[1].extend(unknown());
    body.unknown = unknown();
    let value = seal(&HISTORY, &body.decoded(), BOB);

    let encoded = value.to_cbor();
    let decoded = Presence::from_cbor(&encoded).expect("the value decodes");
    assert_eq!(decoded, value);
    assert_eq!(decoded.to_cbor(), encoded, "byte for byte");

    let opened = open(&decoded, BOB).expect("the value opens");
    assert_eq!(opened, body.decoded());
    assert_eq!(
        opened.to_cbor(),
        body.cbor(),
        "the body, unknown fields included"
    );

    // A value as the server keeps it, with fields of a later version, opens alike.
    let later = Presence::from_cbor(&value_cbor(
        value.device,
        value.channel,
        value.epoch,
        &value.nonce,
        &value.ct,
        unknown(),
    ))
    .expect("a value with unknown fields decodes");
    assert_eq!(open(&later, BOB), Ok(body.decoded()));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_sealed_as_the_spec_says_apart_from_the_crate_opens_to_its_body() {
    let body = Body::of(ALICE, ChannelId::generate());
    assert_eq!(open(&forged_body(&body, ALICE), ALICE), Ok(body.decoded()));

    let empty = Body {
        agents: vec![],
        serves: vec![],
        ..body.clone()
    };
    assert_eq!(
        open(&forged_body(&empty, ALICE), ALICE),
        Ok(empty.decoded()),
        "a device with no agents"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_opens_with_its_signing_device_key_only() {
    let (_, value) = crate_sealed();
    assert!(
        open(&value, BOB).is_err(),
        "refused with another device's key"
    );
}
