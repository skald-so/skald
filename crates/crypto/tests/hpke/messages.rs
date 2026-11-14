//! The `hpke` payload: sealed to one device as `docs/protocol/README.md`, "Scheme `hpke`", says,
//! and opened with its checks 1 and 2; check 3 is the caller's.

use std::num::NonZeroU64;

use skald_crypto::{HpkeKey, open_message, seal_message};
use skald_proto::{
    Cbor, ChannelId, Content, Envelope, Header, HpkePayload, MessageId, NonEmpty, SpaceId,
};

use crate::fixtures::{
    Device, INFO, LABEL, content, header, made, opened, plaintext, sealed, sealed_with, signature,
};

fn bytes(payload: impl AsRef<[u8]>) -> Vec<u8> {
    payload.as_ref().to_vec()
}

/// The spec's content, sealed by Alice's device to Bob's under the spec's header.
fn to_bob() -> Vec<u8> {
    bytes(
        seal_message(
            &header(),
            &content(),
            &Device::Alice.signing(),
            &Device::Bob.hpke_public(),
        )
        .expect("sealed"),
    )
}

/// `header` with `recipients`.
fn with_recipients(recipients: &[Device]) -> Header {
    Header {
        recipients: NonEmpty::from_vec(recipients.iter().map(|device| device.id()).collect()),
        ..header()
    }
}

/// `payload`, decoded.
fn decoded(payload: &[u8]) -> HpkePayload {
    HpkePayload::from_cbor(payload).unwrap()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_sealed_message_opens_with_the_recipients_key() {
    let payload = to_bob();
    let message = open_message(&header(), &payload, Device::Bob.id(), &Device::Bob.hpke())
        .expect("opens with Bob's key");
    assert_eq!(message.sender(), Device::Alice.id());
    let opened = message
        .verify(&Device::Alice.cert())
        .expect("verifies with Alice's certificate");
    assert_eq!(opened, content());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_payload_is_the_specs_hpke_payload() {
    let header = header();
    let content = content();
    let payload = to_bob();
    // The envelope carries it as its payload.
    let envelope = Envelope {
        header: header.clone(),
        payload: payload.clone(),
    };
    assert_eq!(Envelope::from_cbor(&envelope.to_cbor()).unwrap(), envelope);
    // Base mode, single-shot, `info` "skald hpke", `aad` the encoded header; the plaintext signed
    // by `sender` over `["skald hpke", header, content]`. Ed25519 is deterministic, so the
    // plaintext matches byte for byte.
    let HpkePayload { enc, ct } = decoded(&payload);
    let opened = Device::Bob
        .hpke()
        .open(&enc, INFO, &header.to_cbor(), &ct)
        .expect("opens with the spec's info and aad");
    let expected = plaintext(
        Device::Alice,
        &content,
        Device::Alice.sign(LABEL, &[&header.to_cbor(), &content.to_cbor()]),
    );
    assert_eq!(bytes(opened), expected);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_message_takes_a_new_ephemeral_key() {
    assert_ne!(decoded(&to_bob()).enc, decoded(&to_bob()).enc);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_payload_made_apart_opens() {
    // The control for the payloads the tests below change: made as the spec says, it opens.
    let header = header();
    assert_eq!(
        opened(Device::Bob, &header, &made(Device::Bob, &header)),
        Some((Device::Alice.id(), content()))
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn another_devices_key_does_not_open_it() {
    let payload = to_bob();
    let header = header();
    let carol = Device::Carol.hpke();
    assert!(open_message(&header, &payload, Device::Bob.id(), &carol).is_err());
    assert!(open_message(&header, &payload, Device::Carol.id(), &carol).is_err());
    let other = HpkeKey::generate();
    assert!(open_message(&header, &payload, Device::Bob.id(), &other).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn recipients_must_name_the_opening_device_alone() {
    // Each payload is sealed to Bob's key under the header it is opened with, so only check 1
    // refuses it.
    let none = Header {
        recipients: None,
        ..header()
    };
    for header in [
        with_recipients(&[Device::Carol]),
        with_recipients(&[Device::Alice]),
        with_recipients(&[Device::Bob, Device::Carol]),
        with_recipients(&[Device::Carol, Device::Bob]),
        with_recipients(&[Device::Bob, Device::Bob]),
        none,
    ] {
        let payload = made(Device::Bob, &header);
        assert!(
            open_message(&header, &payload, Device::Bob.id(), &Device::Bob.hpke()).is_err(),
            "{:?}",
            header.recipients
        );
    }
}

/// Headers that differ from the spec's in one field.
fn changed_headers() -> Vec<Header> {
    let header = header();
    let mut msg_id = *header.msg_id.as_bytes();
    msg_id[15] ^= 1;
    let mut channel = *header.channel.as_bytes();
    channel[0] ^= 1;
    let mut space = *header.space.as_bytes();
    space[0] ^= 1;
    vec![
        Header {
            msg_id: MessageId::from_bytes(msg_id),
            ..header.clone()
        },
        Header {
            channel: ChannelId::from_bytes(channel),
            ..header.clone()
        },
        Header {
            space: SpaceId::from_bytes(space),
            ..header.clone()
        },
        Header {
            ttl: None,
            ..header.clone()
        },
        Header {
            ttl: Some(60),
            ..header.clone()
        },
        Header {
            v: NonZeroU64::new(2).unwrap(),
            ..header.clone()
        },
    ]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_header_does_not_open() {
    for payload in [to_bob(), made(Device::Bob, &header())] {
        for changed in changed_headers() {
            assert_eq!(opened(Device::Bob, &changed, &payload), None, "{changed:?}");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_enc_or_ct_does_not_open() {
    let header = header();
    let HpkePayload { enc, ct } = decoded(&to_bob());
    for at in [0, 16, 31] {
        let mut changed = enc;
        changed[at] ^= 1;
        let payload = HpkePayload {
            enc: changed,
            ct: ct.clone(),
        };
        assert_eq!(
            opened(Device::Bob, &header, &payload.to_cbor()),
            None,
            "enc at {at}"
        );
    }
    for at in [0, ct.len() / 2, ct.len() - 1] {
        let mut changed = ct.clone();
        changed[at] ^= 1;
        let payload = HpkePayload { enc, ct: changed };
        assert_eq!(
            opened(Device::Bob, &header, &payload.to_cbor()),
            None,
            "ct at {at}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn only_skalds_info_and_the_header_as_aad_open() {
    let header = header();
    let content = content();
    let plaintext = plaintext(
        Device::Alice,
        &content,
        signature(Device::Alice, &header, &content),
    );
    let bob = Device::Bob.hpke_public();
    let encoded = header.to_cbor();
    for (info, aad) in [
        (&b"skald history wrap"[..], &encoded[..]),
        (b"", &encoded[..]),
        (b"skald hpke ", &encoded[..]),
        (INFO, &[][..]),
        (INFO, &encoded[..encoded.len() - 1]),
        (
            INFO,
            &Header {
                ttl: None,
                ..header.clone()
            }
            .to_cbor()[..],
        ),
    ] {
        let payload = sealed_with(&bob, info, aad, &plaintext);
        assert_eq!(
            opened(Device::Bob, &header, &payload),
            None,
            "{info:?}, {aad:?}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_by_another_key_does_not_verify() {
    let header = header();
    let content = content();
    // Alice's device named as `sender`, Carol's signature.
    let payload = sealed(
        Device::Bob,
        &header,
        &plaintext(
            Device::Alice,
            &content,
            signature(Device::Carol, &header, &content),
        ),
    );
    let message = open_message(&header, &payload, Device::Bob.id(), &Device::Bob.hpke())
        .expect("it opens; only the signature is wrong");
    assert_eq!(message.sender(), Device::Alice.id());
    assert!(message.verify(&Device::Alice.cert()).is_err());
    // The certificate whose key made the signature is not `sender`'s.
    let message = open_message(&header, &payload, Device::Bob.id(), &Device::Bob.hpke()).unwrap();
    assert!(message.verify(&Device::Carol.cert()).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn only_the_senders_certificate_verifies() {
    let header = header();
    let payload = to_bob();
    for other in [Device::Bob, Device::Carol] {
        let message =
            open_message(&header, &payload, Device::Bob.id(), &Device::Bob.hpke()).unwrap();
        assert!(message.verify(&other.cert()).is_err(), "{other:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_signature_over_another_label_header_or_content_does_not_verify() {
    let header = header();
    let content = content();
    let encoded = header.to_cbor();
    let other_header = changed_headers().remove(0).to_cbor();
    let other_content = Content {
        hops: content.hops + 1,
        ..content.clone()
    }
    .to_cbor();
    let encoded_content = content.to_cbor();
    for (label, signed_header, signed_content) in [
        ("skald cert", &encoded, &encoded_content),
        ("skald proof", &encoded, &encoded_content),
        ("skald hpke ", &encoded, &encoded_content),
        ("", &encoded, &encoded_content),
        (LABEL, &other_header, &encoded_content),
        (LABEL, &encoded, &other_content),
    ] {
        let signature = Device::Alice.sign(label, &[signed_header.as_slice(), signed_content]);
        let payload = sealed(
            Device::Bob,
            &header,
            &plaintext(Device::Alice, &content, signature),
        );
        let message = open_message(&header, &payload, Device::Bob.id(), &Device::Bob.hpke())
            .expect("it opens; only the signature is wrong");
        assert!(message.verify(&Device::Alice.cert()).is_err(), "{label:?}");
    }
    // A signature over one part more.
    let signature = Device::Alice.sign(LABEL, &[&encoded, &encoded_content, &[]]);
    let payload = sealed(
        Device::Bob,
        &header,
        &plaintext(Device::Alice, &content, signature),
    );
    assert_eq!(opened(Device::Bob, &header, &payload), None);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_payloads_are_refused() {
    let header = header();
    let good = made(Device::Bob, &header);
    let HpkePayload { enc, ct } = decoded(&good);
    let mut trailing = good.clone();
    trailing.push(0);
    // The CBOR of `[enc, ct]` with `enc` of 31 and 33 bytes, `ct` a text string, and one item more.
    let mut short_enc = vec![0x82, 0x58, 31];
    short_enc.extend_from_slice(&enc[..31]);
    short_enc.extend_from_slice(
        &HpkePayload {
            enc,
            ct: ct.clone(),
        }
        .to_cbor()[35..],
    );
    let mut long_enc = vec![0x82, 0x58, 33];
    long_enc.extend_from_slice(&enc);
    long_enc.push(0);
    long_enc.extend_from_slice(
        &HpkePayload {
            enc,
            ct: ct.clone(),
        }
        .to_cbor()[35..],
    );
    let mut three = HpkePayload {
        enc,
        ct: ct.clone(),
    }
    .to_cbor();
    three[0] = 0x83;
    three.push(0);
    for (what, payload) in [
        ("empty", vec![]),
        ("not CBOR", vec![0xff]),
        ("a map", vec![0xa0]),
        ("an empty array", vec![0x80]),
        ("enc alone", {
            let mut one = vec![0x81, 0x58, 32];
            one.extend_from_slice(&enc);
            one
        }),
        ("enc of 31 bytes", short_enc),
        ("enc of 33 bytes", long_enc),
        ("three items", three),
        ("a trailing byte", trailing),
    ] {
        assert_eq!(opened(Device::Bob, &header, &payload), None, "{what}");
    }

    // Sealed as the spec says, but the plaintext is not an `hpke-plaintext`.
    let content = content();
    let signature = signature(Device::Alice, &header, &content);
    let good_plaintext = plaintext(Device::Alice, &content, signature);
    let mut plaintext_trailing = good_plaintext.clone();
    plaintext_trailing.push(0);
    let mut short_signature = good_plaintext.clone();
    // The signature's head `58 40` becomes `58 3f`, and its last byte goes.
    let head = short_signature.len() - 66;
    assert_eq!(short_signature[head..head + 2], [0x58, 0x40]);
    short_signature[head + 1] = 0x3f;
    short_signature.pop();
    let mut no_content = vec![0x83];
    no_content.extend_from_slice(&good_plaintext[1..18]);
    no_content.extend_from_slice(&[0x41, 0x00]);
    no_content.extend_from_slice(&good_plaintext[head..]);
    for (what, plaintext) in [
        ("empty", vec![]),
        ("not CBOR", b"not cbor".to_vec()),
        ("an empty array", vec![0x80]),
        ("a trailing byte", plaintext_trailing),
        ("a signature of 63 bytes", short_signature),
        ("content that is not content", no_content),
        ("the content alone", content.to_cbor()),
    ] {
        let payload = sealed(Device::Bob, &header, &plaintext);
        assert_eq!(opened(Device::Bob, &header, &payload), None, "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn truncated_payloads_are_refused() {
    let header = header();
    for payload in [to_bob(), made(Device::Bob, &header)] {
        for len in 0..payload.len() {
            assert_eq!(
                opened(Device::Bob, &header, &payload[..len]),
                None,
                "cut to {len}"
            );
        }
        // A shorter `ct`, encoded as a whole.
        let HpkePayload { enc, ct } = decoded(&payload);
        for len in 0..ct.len() {
            let cut = HpkePayload {
                enc,
                ct: ct[..len].to_vec(),
            };
            assert_eq!(
                opened(Device::Bob, &header, &cut.to_cbor()),
                None,
                "ct cut to {len}"
            );
        }
    }
    // A plaintext cut short, sealed as the spec says.
    let content = content();
    let plaintext = plaintext(
        Device::Alice,
        &content,
        signature(Device::Alice, &header, &content),
    );
    for len in 0..plaintext.len() {
        let payload = sealed(Device::Bob, &header, &plaintext[..len]);
        assert_eq!(
            opened(Device::Bob, &header, &payload),
            None,
            "plaintext cut to {len}"
        );
    }
}
