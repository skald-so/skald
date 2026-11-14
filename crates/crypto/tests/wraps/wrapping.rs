//! Wrapping an epoch's key: a member device seals it with HPKE in base mode to a browser's `hpke`
//! key or to an account's recovery HPKE key, `info` `skald history wrap`, `aad` the encoded
//! `[channel, epoch, recipient]`, and signs the wrap over `["skald wrap", body]`. A reader drops a
//! wrap unless `ct` opens with its HPKE key and the signature verifies with the certificate of
//! `signer`.

use skald_crypto::{HpkeKey, open_wrap, verify};
use skald_proto::{AccountId, Cbor, ChannelId, DeviceId, HistoryWrap, WrapRecipient};

use crate::fixtures::{
    ACCOUNT, Channel, Fields, INFO, LABEL, OTHER_ACCOUNT, RECOVERY, aad, browser, browser_reader,
    cert, cert_with_key_of, device_id, made_apart, read, readers, recovery_reader, resigned,
    signed, signing_key, unknown, wrap_cbor,
};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_to_a_browser_opens_with_its_key_and_gives_the_epochs_key() {
    let mut channel = Channel::new(&[10, 11]);
    channel.next_epoch();
    let reader = browser_reader(50);
    for epoch in [1, 2] {
        let wrap = channel.wrap(0, epoch, &reader);
        let body = wrap.body();
        assert_eq!(
            (body.channel, body.epoch, body.recipient, body.signer),
            (channel.id, epoch, browser(50), device_id(10)),
            "the wrap's fields"
        );
        assert_eq!(
            read(&wrap, &reader.key),
            Ok(channel.key_bytes(1, epoch)),
            "the key of epoch {epoch}, as the other member holds it"
        );
    }
    assert_ne!(channel.key_bytes(1, 1), channel.key_bytes(1, 2));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_to_a_recovery_key_opens_with_the_key_pair_of_the_recovery_key() {
    let channel = Channel::new(&[12, 13]);
    let wrap = channel.wrap(1, 1, &recovery_reader(ACCOUNT, &RECOVERY));
    let body = wrap.body();
    assert_eq!(
        (body.recipient, body.signer),
        (WrapRecipient::Recovery(ACCOUNT), device_id(13))
    );
    // The key pair comes from the 32-byte recovery key alone, as a device linked with it has it.
    let key = HpkeKey::from_recovery_key(&RECOVERY);
    assert_eq!(read(&wrap, &key), Ok(channel.key_bytes(0, 1)));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_is_sealed_and_signed_as_the_spec_says() {
    let channel = Channel::new(&[14, 15]);
    for reader in readers() {
        let wrap = channel.wrap(0, 1, &reader);
        let body = wrap.body();
        // Apart from the wrapping: the single-shot open with the spec's `info` and `aad`.
        let opened = reader
            .key
            .open(
                &body.enc,
                INFO,
                &aad(channel.id, 1, &reader.recipient),
                &body.ct,
            )
            .expect("ct opens as \"Wraps\" says");
        assert_eq!(opened.as_slice(), channel.key_bytes(1, 1).as_slice());
        verify(
            &signing_key(14).public_key(),
            LABEL,
            &[wrap.body_bytes()],
            wrap.signature(),
        )
        .expect("signed by the member device over [\"skald wrap\", body]");

        let again = channel.wrap(0, 1, &reader);
        assert_ne!(again.body().enc, body.enc, "a new ephemeral key each time");
        assert_ne!(again.body().ct, body.ct);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_made_apart_from_the_crate_as_the_spec_says_opens_and_gives_its_key() {
    let channel = ChannelId::generate();
    let key = [0x4b; 16];
    for reader in readers() {
        for unknown in [vec![], unknown()] {
            let wrap = made_apart(16, channel, 3, &reader, &key, unknown);
            assert_eq!(read(&wrap, &reader.key), Ok(key));
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_opened_with_another_recipients_key_is_refused() {
    let channel = Channel::new(&[17, 18]);
    let browser_wrap = channel.wrap(0, 1, &browser_reader(50));
    let recovery_wrap = channel.wrap(0, 1, &recovery_reader(ACCOUNT, &RECOVERY));
    let mut other_recovery = RECOVERY;
    other_recovery[0] ^= 1;
    for (what, wrap, key) in [
        ("another browser", &browser_wrap, browser_reader(51).key),
        (
            "a recovery key",
            &browser_wrap,
            HpkeKey::from_recovery_key(&RECOVERY),
        ),
        (
            "another recovery key",
            &recovery_wrap,
            HpkeKey::from_recovery_key(&other_recovery),
        ),
        ("a browser", &recovery_wrap, browser_reader(50).key),
        (
            "the recovery key itself as a private key",
            &recovery_wrap,
            HpkeKey::from_bytes(&RECOVERY),
        ),
    ] {
        assert!(open_wrap(wrap, &key).is_err(), "{what}");
        assert!(read(wrap, &key).is_err(), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_with_a_changed_channel_epoch_or_recipient_is_refused() {
    let mut channel = Channel::new(&[19, 20]);
    channel.next_epoch();
    channel.next_epoch();
    for reader in readers() {
        let wrap = channel.wrap(0, 2, &reader);
        assert_eq!(read(&wrap, &reader.key), Ok(channel.key_bytes(1, 2)));
        // The same ID bytes, as a recipient of the other kind.
        let other_kind = match reader.recipient {
            WrapRecipient::Browser(device) => {
                WrapRecipient::Recovery(AccountId::from_bytes(*device.as_bytes()))
            }
            WrapRecipient::Recovery(account) => {
                WrapRecipient::Browser(DeviceId::from_bytes(*account.as_bytes()))
            }
        };
        let changes: [(&str, Change); 7] = [
            (
                "another channel",
                Box::new(|fields| fields.channel = ChannelId::generate()),
            ),
            ("epoch 1", Box::new(|fields| fields.epoch = 1)),
            ("epoch 3", Box::new(|fields| fields.epoch = 3)),
            (
                "another browser",
                Box::new(|fields| fields.recipient = WrapRecipient::Browser(device_id(51))),
            ),
            (
                "another account",
                Box::new(|fields| fields.recipient = WrapRecipient::Recovery(OTHER_ACCOUNT)),
            ),
            (
                "the recipient's ID as the other kind",
                Box::new(move |fields| fields.recipient = other_kind),
            ),
            (
                "another signer",
                Box::new(|fields| fields.signer = device_id(20)),
            ),
        ];
        for (what, change) in changes {
            let mut fields = Fields::of(&wrap);
            change(&mut fields);
            assert_ne!(
                fields.encode(vec![]),
                wrap.body_bytes(),
                "{what} changes the body"
            );
            // With the original signature.
            let unsigned = signed_with(&fields, *wrap.signature());
            assert!(read(&unsigned, &reader.key).is_err(), "{what}");
            if what == "another signer" {
                continue;
            }
            // Re-signed by the signer, so only the seal can tell.
            let resigned = resigned(19, &fields);
            assert!(
                open_wrap(&resigned, &reader.key).is_err(),
                "{what}, re-signed"
            );
            assert!(read(&resigned, &reader.key).is_err(), "{what}, re-signed");
        }
    }
}

/// A change to a wrap's fields.
type Change = Box<dyn Fn(&mut Fields)>;

/// `fields` with `signature`, whoever made it.
fn signed_with(fields: &Fields, signature: [u8; 64]) -> HistoryWrap {
    HistoryWrap::from_cbor(&wrap_cbor(&fields.encode(vec![]), &signature)).expect("a wrap")
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_with_a_changed_enc_or_ct_is_refused() {
    let channel = Channel::new(&[21, 22]);
    for reader in readers() {
        let wrap = channel.wrap(0, 1, &reader);
        let fields = Fields::of(&wrap);
        let mut changed: Vec<(String, Fields)> = Vec::new();
        for at in [0, 15, 31] {
            let mut other = fields.clone();
            other.enc[at] ^= 1;
            changed.push((format!("enc changed at {at}"), other));
        }
        for at in [0, fields.ct.len() / 2, fields.ct.len() - 1] {
            let mut other = fields.clone();
            other.ct[at] ^= 1;
            changed.push((format!("ct changed at {at}"), other));
        }
        for len in 0..fields.ct.len() {
            let mut other = fields.clone();
            other.ct.truncate(len);
            changed.push((format!("ct cut to {len}"), other));
        }
        let mut longer = fields.clone();
        longer.ct.push(0);
        changed.push(("ct with a byte more".to_owned(), longer));
        let mut other = fields.clone();
        other.enc = channel.wrap(0, 1, &reader).body().enc.to_vec();
        changed.push(("the enc of another wrap".to_owned(), other));
        for (what, fields) in changed {
            let wrap = resigned(21, &fields);
            assert!(open_wrap(&wrap, &reader.key).is_err(), "{what}");
            assert!(read(&wrap, &reader.key).is_err(), "{what}");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_signed_by_another_key_or_over_another_label_is_refused() {
    let channel = Channel::new(&[23, 24]);
    for reader in readers() {
        let wrap = channel.wrap(0, 1, &reader);
        let body = wrap.body_bytes();
        let mut flipped = *wrap.signature();
        flipped[10] ^= 1;
        let forged = [
            ("signed by the other member", signed(24, LABEL, body)),
            (
                "signed by a device outside the group",
                signed(60, LABEL, body),
            ),
            (
                "signed over \"skald history\"",
                signed(23, "skald history", body),
            ),
            ("signed over \"skald hpke\"", signed(23, "skald hpke", body)),
            ("signed over \"skald link\"", signed(23, "skald link", body)),
            (
                "a changed signature",
                signed_with(&Fields::of(&wrap), flipped),
            ),
        ];
        for (what, forged) in forged {
            assert_eq!(
                open_wrap(&forged, &reader.key)
                    .expect("ct still opens")
                    .signer(),
                device_id(23)
            );
            assert!(
                open_wrap(&forged, &reader.key)
                    .expect("ct still opens")
                    .verify(&cert(23))
                    .is_err(),
                "{what}"
            );
            assert!(read(&forged, &reader.key).is_err(), "{what}");
        }

        // The signature is good, but the certificate given is not `signer`'s.
        for other in [24, 60] {
            assert!(
                open_wrap(&wrap, &reader.key)
                    .expect("the wrap opens")
                    .verify(&cert(other))
                    .is_err(),
                "the certificate of device {other}"
            );
        }
        assert!(
            open_wrap(&wrap, &reader.key)
                .expect("the wrap opens")
                .verify(&cert_with_key_of(24, 23))
                .is_err(),
            "a certificate of another device with the signer's key"
        );
        let verified = open_wrap(&wrap, &reader.key)
            .expect("the wrap opens")
            .verify(&cert(23))
            .expect("the signer's certificate verifies it");
        assert_eq!(*verified.to_bytes(), channel.key_bytes(1, 1));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_wrap_whose_sealed_key_is_not_16_bytes_is_refused() {
    let channel = ChannelId::generate();
    for reader in readers() {
        assert_eq!(
            read(
                &made_apart(25, channel, 1, &reader, &[0x4b; 16], vec![]),
                &reader.key
            ),
            Ok([0x4b; 16]),
            "16 bytes open"
        );
        for len in [0, 1, 15, 17, 32] {
            let wrap = made_apart(25, channel, 1, &reader, &vec![0x4b; len], vec![]);
            assert!(read(&wrap, &reader.key).is_err(), "{len} bytes");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn no_wrap_is_made_of_epoch_zero_or_to_a_key_of_small_order() {
    let channel = Channel::new(&[26, 27]);
    let key = channel.key(0, 1);
    let reader = browser_reader(50);
    assert!(
        key.wrap(
            channel.id,
            0,
            reader.recipient,
            &reader.public,
            &signing_key(26)
        )
        .is_err(),
        "epoch 0"
    );
    let mut one = [0; 32];
    one[0] = 1;
    for (what, public) in [("the point 0", [0; 32]), ("the point 1", one)] {
        assert!(
            key.wrap(channel.id, 1, reader.recipient, &public, &signing_key(26))
                .is_err(),
            "{what}"
        );
    }
    assert!(
        key.wrap(
            channel.id,
            1,
            WrapRecipient::Recovery(ACCOUNT),
            &HpkeKey::from_recovery_key(&RECOVERY).public_key(),
            &signing_key(26),
        )
        .is_ok()
    );
}
