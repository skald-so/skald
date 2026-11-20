//! The reader checks of `docs/protocol/agents.md`, "Presence", that need nothing of the reader's
//! state: check 1, the body naming the value's `device`, `channel` and `epoch` and its signature by
//! the given device key, of check 2, `expires` against the given time, check 4, and of check 5, no
//! agent a person or repeated and no queue repeated. Each refused value differs from one that opens
//! only in what the test names.

use skald_crypto::PresenceError;
use skald_proto::{ChannelId, Presence, QueueId};

use crate::devices::{NOW, device_id};
use crate::fields::{signed_cbor, unknown};
use crate::fixtures::*;

/// Alice's body in a new channel, and the value sealed as the spec says that opens to it.
fn opening() -> (Body, Presence) {
    let body = Body::of(ALICE, ChannelId::generate());
    let value = forged_body(&body, ALICE);
    assert_eq!(
        open(&value, ALICE),
        Ok(body.decoded()),
        "the value as made opens"
    );
    (body, value)
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_under_another_epochs_key_is_refused() {
    let (body, value) = crate_sealed();
    assert!(
        open_at(&OTHER_HISTORY, &value, ALICE, NOW).is_err(),
        "refused with another epoch's history key"
    );
    let other = forged(
        &OTHER_HISTORY,
        (body.device, body.channel, body.epoch),
        &body.signed(ALICE, LABEL),
    );
    assert!(
        open(&other, ALICE).is_err(),
        "refused: sealed under another epoch's key"
    );
    let raw = sealed_with(
        &HISTORY,
        &aad(body.device, body.channel, body.epoch),
        (body.device, body.channel, body.epoch),
        &padded(&body.signed(ALICE, LABEL)),
    );
    assert!(
        open(&raw, ALICE).is_err(),
        "refused: sealed under the history key itself"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_with_another_device_channel_or_epoch_in_its_associated_data_is_refused() {
    let (body, value) = crate_sealed();

    let mut device = value.clone();
    device.device = device_id(BOB);
    let mut channel = value.clone();
    channel.channel = ChannelId::generate();
    let mut epoch = value.clone();
    epoch.epoch = 2;
    for (what, changed) in [("device", device), ("channel", channel), ("epoch", epoch)] {
        assert!(
            open(&changed, ALICE).is_err(),
            "refused: another {what} in the value"
        );
    }

    // Sealed with associated data that differs from the value's own fields.
    let sealed_fields = (body.device, body.channel, body.epoch);
    let plaintext = padded(&body.signed(ALICE, LABEL));
    for (what, aad) in [
        (
            "another device",
            aad(device_id(BOB), body.channel, body.epoch),
        ),
        (
            "another channel",
            aad(body.device, ChannelId::generate(), body.epoch),
        ),
        ("another epoch", aad(body.device, body.channel, 2)),
        (
            "another label",
            crate::cbor::Value::Array(vec![
                crate::cbor::text("skald history entry"),
                crate::cbor::bytes(body.device.as_bytes()),
                crate::cbor::bytes(body.channel.as_bytes()),
                crate::cbor::Value::Uint(body.epoch),
            ])
            .encode(),
        ),
    ] {
        let forged = sealed_with(&derived(&HISTORY), &aad, sealed_fields, &plaintext);
        assert!(
            open(&forged, ALICE).is_err(),
            "refused: {what} in the associated data"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_body_naming_another_device_channel_or_epoch_than_its_value_is_refused() {
    let (body, _) = opening();
    let value_fields = (body.device, body.channel, body.epoch);
    for (what, changed) in [
        (
            "another device",
            Body {
                device: device_id(BOB),
                ..body.clone()
            },
        ),
        (
            "another channel",
            Body {
                channel: ChannelId::generate(),
                ..body.clone()
            },
        ),
        (
            "another epoch",
            Body {
                epoch: 2,
                ..body.clone()
            },
        ),
    ] {
        let value = forged(&HISTORY, value_fields, &changed.signed(ALICE, LABEL));
        assert!(
            open(&value, ALICE).is_err(),
            "refused: a body naming {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_nonzero_byte_after_the_plaintext_is_refused() {
    let (body, _) = opening();
    let fields = (body.device, body.channel, body.epoch);
    let key = derived(&HISTORY);
    let aad = aad(body.device, body.channel, body.epoch);
    let signed = body.signed(ALICE, LABEL);
    let clean = padded(&signed);

    let mut changes: Vec<(&str, Vec<u8>)> = Vec::new();
    for (what, at) in [
        ("right after the plaintext", signed.len()),
        ("amid the padding", (signed.len() + BLOCK) / 2),
        ("last", BLOCK - 1),
    ] {
        let mut changed = clean.clone();
        changed[at] = 1;
        changes.push((what, changed));
    }
    let mut longer = clean.clone();
    longer.extend([0x80].iter().chain([0; BLOCK - 1].iter()));
    changes.push(("in a further block", longer));
    changes.push(("with no padding", [signed.as_slice(), &[1]].concat()));
    for (what, plaintext) in changes {
        let value = sealed_with(&key, &aad, fields, &plaintext);
        assert!(
            open(&value, ALICE).is_err(),
            "refused: a nonzero byte {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_broken_signature_another_label_or_another_devices_signature_is_refused() {
    let (body, _) = opening();
    let fields = (body.device, body.channel, body.epoch);
    let encoded = body.cbor();
    let signature = crate::devices::signing_key(ALICE).sign(LABEL, &[&encoded]);

    let mut broken = signature;
    broken[10] ^= 1;
    for (what, plaintext) in [
        ("a broken signature", signed_cbor(&encoded, &broken)),
        (
            "a signature under another label",
            body.signed(ALICE, "skald history"),
        ),
        ("a signature by another device", body.signed(BOB, LABEL)),
        ("no signature", signed_cbor(&encoded, &[0; 64])),
    ] {
        let value = forged(&HISTORY, fields, &plaintext);
        assert!(open(&value, ALICE).is_err(), "refused: {what}");
    }

    // Bob signs a body naming Bob, in a value of Bob's: it opens with Bob's key, not Alice's.
    let bobs = Body {
        device: device_id(BOB),
        ..body.clone()
    };
    let value = forged_body(&bobs, BOB);
    assert_eq!(open(&value, BOB), Ok(bobs.decoded()));
    assert!(
        open(&value, ALICE).is_err(),
        "refused with another device's key"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_past_its_expires_is_refused_as_expired() {
    let (body, value) = opening();
    assert_eq!(
        open_at(&HISTORY, &value, ALICE, EXPIRES - 60),
        Ok(body.decoded()),
        "before expires"
    );
    for now in [EXPIRES + 1, EXPIRES + 3600] {
        let refused = open_at(&HISTORY, &value, ALICE, now);
        assert!(
            matches!(refused, Err(PresenceError::Expired { .. })),
            "refused as expired at {now}: {refused:?}"
        );
    }

    let sealed = seal(&HISTORY, &body.decoded(), ALICE);
    assert!(
        matches!(
            open_at(&HISTORY, &sealed, ALICE, EXPIRES + 1),
            Err(PresenceError::Expired { .. })
        ),
        "a value the crate sealed, past expires"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_listing_a_person_as_an_agent_is_refused() {
    let (body, _) = opening();
    for agents in [
        vec![status(PERSON, AVAILABLE)],
        vec![status(AGENT, AVAILABLE), status(PERSON, BUSY)],
    ] {
        let changed = Body {
            agents,
            ..body.clone()
        };
        assert!(
            open(&forged_body(&changed, ALICE), ALICE).is_err(),
            "refused: a person among the agents"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_listing_an_agent_twice_is_refused() {
    let (body, _) = opening();
    let mut twice = status_with(AGENT, AVAILABLE, "another note", "codex");
    twice.extend(unknown());
    for agents in [
        vec![status(AGENT, BUSY), status(AGENT, BUSY)],
        vec![status(AGENT, BUSY), status(OTHER_AGENT, OFFLINE), twice],
    ] {
        let changed = Body {
            agents,
            ..body.clone()
        };
        assert!(
            open(&forged_body(&changed, ALICE), ALICE).is_err(),
            "refused: an agent listed twice"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_listing_a_queue_twice_is_refused() {
    let (body, _) = opening();
    let id = QueueId::generate();
    let two = Body {
        serves: vec![
            queue(id, [FULL, OFF, OFF]),
            queue(QueueId::generate(), [FULL, OFF, OFF]),
        ],
        ..body.clone()
    };
    assert_eq!(
        open(&forged_body(&two, ALICE), ALICE),
        Ok(two.decoded()),
        "two queues open"
    );
    let twice = Body {
        serves: vec![
            queue(id, [FULL, OFF, OFF]),
            queue(id, [FULL, APPROVE, READ_ONLY]),
        ],
        ..body.clone()
    };
    assert!(
        open(&forged_body(&twice, ALICE), ALICE).is_err(),
        "refused: a queue listed twice"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_with_a_changed_or_truncated_ciphertext_or_nonce_is_refused_without_a_panic() {
    let (_, value) = opening();
    let mut changes: Vec<(String, Presence)> = Vec::new();
    let mut nonce = value.clone();
    nonce.nonce[0] ^= 1;
    changes.push(("a changed nonce".into(), nonce));
    for at in [0, value.ct.len() / 2, value.ct.len() - 1] {
        let mut ct = value.clone();
        ct.ct[at] ^= 0x80;
        changes.push((format!("ct changed at {at}"), ct));
    }
    for length in [0, 1, 15, 16, 17, BLOCK, value.ct.len() - 1] {
        let mut ct = value.clone();
        ct.ct.truncate(length);
        changes.push((format!("ct truncated to {length}"), ct));
    }
    let mut longer = value.clone();
    longer.ct.push(0);
    changes.push(("a longer ct".into(), longer));
    for (what, changed) in changes {
        assert!(open(&changed, ALICE).is_err(), "refused: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_value_holding_no_presence_plaintext_is_refused_without_a_panic() {
    let (body, _) = opening();
    let fields = (body.device, body.channel, body.epoch);
    let signed = body.signed(ALICE, LABEL);
    let malformed = Body {
        epoch: 0,
        ..body.clone()
    };
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("nothing", vec![]),
        ("zeros only", vec![0; BLOCK]),
        ("the body unsigned", body.cbor()),
        (
            "a body that breaks its CDDL",
            malformed.signed(ALICE, LABEL),
        ),
        ("a signed object that is no array", signed[1..].to_vec()),
    ];
    // The padding's zeros complete a plaintext cut only before zeros, which is then the whole
    // plaintext: each cut ends before the last nonzero byte.
    let last_nonzero = signed.iter().rposition(|&byte| byte != 0).unwrap();
    for length in [1, signed.len() / 2, signed.len() - 1] {
        let length = length.min(last_nonzero);
        cases.push(("a truncated plaintext", signed[..length].to_vec()));
    }
    for (what, plaintext) in cases {
        let value = forged(&HISTORY, fields, &plaintext);
        assert!(open(&value, ALICE).is_err(), "refused: {what}");
    }
}
