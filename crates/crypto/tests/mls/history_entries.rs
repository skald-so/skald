//! History entries: `docs/protocol/membership.md`, "History entry". The sending device signs a copy
//! of its content, `["skald history", body]`, and seals it with AES-128-GCM under its epoch's key, a
//! random nonce and the associated data `["skald history entry", channel, epoch]`. A reader
//! decrypts it, checks that the body's `channel` is the entry's and verifies the signature with the
//! certificate of `sender`, which the caller has checked.

use skald_crypto::{HistoryKey, verify};
use skald_proto::{
    Cbor, ChannelId, Content, HistoryBody, HistoryEntry, HistoryPlaintext, MessageId,
};

use crate::fixtures::{admitted, device_id, signing_key};
use crate::history::{
    LABEL, SENT, Sealed, body, body_cbor, content, entry_aad, entry_cbor, forged_entry, key,
    next_epoch, open, plaintext_cbor, read, signed, unknown,
};
use crate::steps::{channel_of, creator, history_key};

/// The certificate of device `seed`.
fn cert(seed: u8) -> &'static skald_proto::Cert {
    &admitted()[usize::from(seed)]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_opens_with_its_epochs_key_and_gives_its_sender_and_the_signed_body() {
    let channel = ChannelId::generate();
    let seeds = [80, 81, 82];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    next_epoch(&seeds, &mut states, channel);
    let msg_id = MessageId::generate();
    let sent = body(81, channel, msg_id);

    let entry = key(81, &states[1], channel, 2)
        .seal_entry(2, &sent, &signing_key(81))
        .expect("an entry");
    assert_eq!(
        (entry.channel, entry.epoch),
        (channel, 2),
        "the entry's fields"
    );
    for (at, &seed) in seeds.iter().enumerate() {
        let message = key(seed, &states[at], channel, 2)
            .open_entry(&entry)
            .unwrap_or_else(|error| panic!("device {seed} opens the entry: {error:?}"));
        assert_eq!(message.sender(), device_id(81), "device {seed}: the sender");
        let got = message
            .verify(cert(81))
            .unwrap_or_else(|error| panic!("device {seed} verifies the entry: {error:?}"));
        assert_eq!(got, sent, "device {seed} gets the body");
        assert_eq!(
            (got.sender, got.channel, got.msg_id, got.timestamp),
            (device_id(81), channel, msg_id, SENT)
        );
        assert_eq!(got.content, Content::from_cbor(&content()).unwrap());
    }

    let encoded = entry.to_cbor();
    let decoded = HistoryEntry::from_cbor(&encoded).expect("the entry decodes");
    assert_eq!(decoded, entry, "the entry round-trips");
    assert_eq!(decoded.to_cbor(), encoded, "byte for byte");
    // An entry as the server keeps it, with fields of a later version, reads alike.
    let later = HistoryEntry::from_cbor(&entry_cbor(
        Sealed {
            channel,
            epoch: 2,
            nonce: &entry.nonce,
            ciphertext: &entry.ciphertext,
        },
        unknown(),
    ))
    .expect("an entry with unknown fields decodes");
    assert_eq!(read(&key(80, &states[0], channel, 2), &later), Ok(sent));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_is_its_signed_body_sealed_with_aes_gcm_under_its_epochs_key_as_the_spec_says() {
    let channel = ChannelId::generate();
    let states = channel_of(channel, 83, &[84]);
    let sent = body(83, channel, MessageId::generate());
    let entry = key(83, &states[0], channel, 1)
        .seal_entry(1, &sent, &signing_key(83))
        .expect("an entry");

    // Apart from the crate: AES-128-GCM under epoch 1's key with the spec's associated data.
    let plaintext = open(
        &history_key(84, &states[1], channel, 1),
        &entry.nonce,
        &entry_aad(channel, 1),
        &entry.ciphertext,
    )
    .expect("the entry opens as \"History entry\" says");
    let plaintext = HistoryPlaintext::from_cbor(&plaintext).expect("a history plaintext");
    assert_eq!(plaintext.body(), &sent);
    assert_eq!(plaintext.body_bytes(), sent.to_cbor().as_slice());
    verify(
        &signing_key(83).public_key(),
        LABEL,
        &[plaintext.body_bytes()],
        plaintext.signature(),
    )
    .expect("signed by the sender over [\"skald history\", body]");

    let again = key(83, &states[0], channel, 1)
        .seal_entry(1, &sent, &signing_key(83))
        .expect("an entry");
    assert_ne!(again.nonce, entry.nonce, "a fresh random nonce each time");
    assert_ne!(again.ciphertext, entry.ciphertext);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_under_another_key_or_with_another_channel_or_epoch_is_refused() {
    let channel = ChannelId::generate();
    let other = ChannelId::generate();
    let seeds = [85, 86];
    let mut states = channel_of(channel, seeds[0], &seeds[1..]);
    next_epoch(&seeds, &mut states, channel);
    let other_state = creator(87, other);
    let sent = body(85, channel, MessageId::generate());
    let entry = key(85, &states[0], channel, 1)
        .seal_entry(1, &sent, &signing_key(85))
        .expect("an entry");
    let reader = key(86, &states[1], channel, 1);
    assert_eq!(
        read(&reader, &entry),
        Ok(sent.clone()),
        "the entry as made reads"
    );

    for (what, key) in [
        ("epoch 2's key", key(86, &states[1], channel, 2)),
        ("another channel's key", key(87, &other_state, other, 0)),
        ("a key of no epoch", HistoryKey::from_bytes(&[0x13; 16])),
    ] {
        assert!(key.open_entry(&entry).is_err(), "refused with {what}");
    }
    let forged = forged_entry(&[0x13; 16], channel, 1, &signed(85, LABEL, &sent.to_cbor()));
    assert!(
        read(&reader, &forged).is_err(),
        "refused: an entry under another key"
    );

    let mut with_channel = entry.clone();
    with_channel.channel = other;
    let mut with_epoch = entry.clone();
    with_epoch.epoch = 2;
    for (what, changed, key) in [
        ("another channel", &with_channel, &reader),
        ("another epoch", &with_epoch, &reader),
        (
            "another epoch, read with its epoch's key",
            &with_epoch,
            &key(86, &states[1], channel, 2),
        ),
    ] {
        assert!(
            key.open_entry(changed).is_err(),
            "refused: an entry with {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_with_a_changed_nonce_or_ciphertext_is_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let states = channel_of(channel, 88, &[89]);
    let entry = key(88, &states[0], channel, 1)
        .seal_entry(
            1,
            &body(88, channel, MessageId::generate()),
            &signing_key(88),
        )
        .expect("an entry");
    let reader = key(89, &states[1], channel, 1);

    let mut changes: Vec<(&str, HistoryEntry)> = Vec::new();
    let mut nonce = entry.clone();
    nonce.nonce[0] ^= 1;
    changes.push(("a changed nonce", nonce));
    for at in [0, entry.ciphertext.len() / 2, entry.ciphertext.len() - 1] {
        let mut ciphertext = entry.clone();
        ciphertext.ciphertext[at] ^= 0x80;
        changes.push(("a changed ciphertext", ciphertext));
    }
    for length in [0, 1, 15, 16, entry.ciphertext.len() - 1] {
        let mut truncated = entry.clone();
        truncated.ciphertext.truncate(length);
        changes.push(("a truncated ciphertext", truncated));
    }
    let mut longer = entry.clone();
    longer.ciphertext.push(0);
    changes.push(("a longer ciphertext", longer));
    for (what, changed) in &changes {
        assert!(reader.open_entry(changed).is_err(), "refused: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_whose_body_is_of_another_channel_is_refused() {
    let channel = ChannelId::generate();
    let other = ChannelId::generate();
    let states = channel_of(channel, 80, &[81]);
    let key_bytes = history_key(81, &states[1], channel, 1);
    let reader = key(81, &states[1], channel, 1);

    // Signed by its sender and sealed under the channel's key, but the body is of another channel.
    let body = body_cbor(
        device_id(80),
        other,
        MessageId::generate(),
        &content(),
        vec![],
    );
    let forged = forged_entry(&key_bytes, channel, 1, &signed(80, LABEL, &body));
    assert!(
        read(&reader, &forged).is_err(),
        "refused: a body of another channel"
    );

    // The same, of its own channel, reads: only the body's channel differs.
    let body = body_cbor(
        device_id(80),
        channel,
        MessageId::generate(),
        &content(),
        vec![],
    );
    let genuine = forged_entry(&key_bytes, channel, 1, &signed(80, LABEL, &body));
    assert_eq!(
        read(&reader, &genuine),
        Ok(HistoryBody::from_cbor(&body).unwrap())
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_signed_by_another_device_or_over_another_label_is_refused() {
    let channel = ChannelId::generate();
    let states = channel_of(channel, 82, &[83, 84]);
    let key_bytes = history_key(83, &states[1], channel, 1);
    let reader = key(83, &states[1], channel, 1);
    let body = body_cbor(
        device_id(82),
        channel,
        MessageId::generate(),
        &content(),
        vec![],
    );

    for (what, plaintext) in [
        ("signed by another device", signed(84, LABEL, &body)),
        ("signed over another label", signed(82, "skald hpke", &body)),
        ("signed over another label", signed(82, "skald wrap", &body)),
        (
            "signed over no label",
            plaintext_cbor(&body, &signing_key(82).sign("", &[body.as_slice()])),
        ),
        ("with a signature of zeros", plaintext_cbor(&body, &[0; 64])),
    ] {
        let forged = forged_entry(&key_bytes, channel, 1, &plaintext);
        let message = reader
            .open_entry(&forged)
            .unwrap_or_else(|error| panic!("{what}: the entry opens: {error:?}"));
        assert_eq!(message.sender(), device_id(82), "{what}: the body's sender");
        assert!(
            message.verify(cert(82)).is_err(),
            "refused: an entry {what}"
        );
    }
    // Device 84's signature verifies with its own certificate, but device 84 is not `sender`.
    let forged = forged_entry(&key_bytes, channel, 1, &signed(84, LABEL, &body));
    assert!(
        reader
            .open_entry(&forged)
            .expect("the entry opens")
            .verify(cert(84))
            .is_err(),
        "refused: verified with the signer's certificate, not the sender's"
    );

    // A genuine entry verifies only with its sender's certificate.
    let entry = key(82, &states[0], channel, 1)
        .seal_entry(1, &HistoryBody::from_cbor(&body).unwrap(), &signing_key(82))
        .expect("an entry");
    for seed in [83, 84] {
        assert!(
            reader
                .open_entry(&entry)
                .expect("the entry opens")
                .verify(cert(seed))
                .is_err(),
            "refused: verified with device {seed}'s certificate, not the sender's"
        );
    }
    assert!(reader.open_entry(&entry).unwrap().verify(cert(82)).is_ok());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_entry_whose_plaintext_is_not_a_signed_body_is_refused_without_a_panic() {
    let channel = ChannelId::generate();
    let key_bytes = [0x4b; 16];
    let reader = HistoryKey::from_bytes(&key_bytes);
    let body = body_cbor(
        device_id(85),
        channel,
        MessageId::generate(),
        &content(),
        vec![],
    );
    let plaintext = signed(85, LABEL, &body);

    let mut plaintexts: Vec<(&str, Vec<u8>)> = vec![
        ("one byte", vec![0]),
        ("not CBOR", vec![0xff; 40]),
        ("a body unsigned", body.clone()),
        ("content in place of a body", signed(85, LABEL, &content())),
        ("a signature of 63 bytes", plaintext_cbor(&body, &[0; 63])),
        ("trailing bytes", [plaintext.clone(), vec![0]].concat()),
    ];
    for length in [1, plaintext.len() / 2, plaintext.len() - 1] {
        plaintexts.push(("a truncated plaintext", plaintext[..length].to_vec()));
    }
    for (what, plaintext) in &plaintexts {
        let forged = forged_entry(&key_bytes, channel, 1, plaintext);
        assert!(read(&reader, &forged).is_err(), "refused: {what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn no_entry_of_epoch_zero_is_made() {
    let channel = ChannelId::generate();
    let state = creator(86, channel);
    assert!(
        key(86, &state, channel, 0)
            .seal_entry(
                0,
                &body(86, channel, MessageId::generate()),
                &signing_key(86)
            )
            .is_err(),
        "epoch 0 holds no messages"
    );
}
