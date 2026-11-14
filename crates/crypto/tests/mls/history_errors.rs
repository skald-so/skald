//! The error each refusal of a history link or entry gives, and what each error says.

use skald_crypto::{HistoryError, HistoryKey};
use skald_proto::{ChannelId, MessageId};

use crate::fixtures::{admitted, device_id, signing_key};
use crate::history::{LABEL, body, body_cbor, content, forged_entry, forged_link, signed};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_refusal_gives_its_error() {
    let channel = ChannelId::generate();
    let bytes = [3; 16];
    let key = HistoryKey::from_bytes(&bytes);
    let other = HistoryKey::from_bytes(&[4; 16]);

    let refused = key.seal_link(channel, 2, 2, &other);
    assert_eq!(refused.unwrap_err(), HistoryError::Bounds);
    let link = key.seal_link(channel, 2, 1, &other).unwrap();
    assert_eq!(other.open_link(&link).unwrap_err(), HistoryError::Open);
    let mut bounds = link.clone();
    bounds.prev = 0;
    assert_eq!(key.open_link(&bounds).unwrap_err(), HistoryError::Bounds);
    let short = forged_link(&bytes, channel, 2, 1, &[5; 15]);
    assert_eq!(key.open_link(&short).unwrap_err(), HistoryError::Malformed);

    let sent = body(6, channel, MessageId::generate());
    let refused = key.seal_entry(0, &sent, &signing_key(6));
    assert_eq!(refused.unwrap_err(), HistoryError::Bounds);
    let entry = key.seal_entry(1, &sent, &signing_key(6)).unwrap();
    assert_eq!(other.open_entry(&entry).unwrap_err(), HistoryError::Open);
    let malformed = forged_entry(&bytes, channel, 1, &[0]);
    assert_eq!(
        key.open_entry(&malformed).unwrap_err(),
        HistoryError::Malformed
    );
    let foreign = body_cbor(
        device_id(6),
        ChannelId::generate(),
        MessageId::generate(),
        &content(),
        vec![],
    );
    let foreign = forged_entry(&bytes, channel, 1, &signed(6, LABEL, &foreign));
    assert_eq!(
        key.open_entry(&foreign).unwrap_err(),
        HistoryError::OtherChannel
    );

    let cert = |seed: u8| &admitted()[usize::from(seed)];
    let message = || key.open_entry(&entry).unwrap();
    assert_eq!(
        message().verify(cert(8)).unwrap_err(),
        HistoryError::NotSender
    );
    let forged = key.seal_entry(1, &sent, &signing_key(8)).unwrap();
    assert_eq!(
        key.open_entry(&forged)
            .unwrap()
            .verify(cert(6))
            .unwrap_err(),
        HistoryError::Signature
    );
    assert_eq!(message().verify(cert(6)), Ok(sent));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_history_error_says_what_failed() {
    for (error, message) in [
        (
            HistoryError::Bounds,
            "an epoch or prev outside the key chain's bounds",
        ),
        (HistoryError::Open, "it does not decrypt with the key"),
        (
            HistoryError::Malformed,
            "it holds no history key or plaintext, or the body is malformed",
        ),
        (
            HistoryError::OtherChannel,
            "the body is of another channel than the entry",
        ),
        (
            HistoryError::NotSender,
            "the certificate is not the sender's or the signer's",
        ),
        (
            HistoryError::Signature,
            "the signature does not verify with the sender's or the signer's key",
        ),
    ] {
        assert_eq!(error.to_string(), message);
    }
}
