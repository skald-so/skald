//! The error each refusal of a wrap gives, and what the error of a small-order key says.

use skald_crypto::{HistoryError, HistoryKey, open_wrap};
use skald_proto::ChannelId;

use crate::fixtures::{browser_reader, cert, made_apart, signed, signing_key};

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_refusal_of_a_wrap_gives_its_error() {
    let channel = ChannelId::generate();
    let key = HistoryKey::from_bytes(&[3; 16]);
    let reader = browser_reader(50);
    let wrap = |epoch, public: &[u8; 32]| {
        key.wrap(channel, epoch, reader.recipient, public, &signing_key(6))
    };
    assert_eq!(wrap(0, &reader.public).unwrap_err(), HistoryError::Bounds);
    assert_eq!(wrap(1, &[0; 32]).unwrap_err(), HistoryError::RecipientKey);

    let made = wrap(1, &reader.public).unwrap();
    let other = browser_reader(51).key;
    assert_eq!(open_wrap(&made, &other).unwrap_err(), HistoryError::Open);
    let short = made_apart(6, channel, 1, &reader, &[3; 15], vec![]);
    assert_eq!(
        open_wrap(&short, &reader.key).unwrap_err(),
        HistoryError::Malformed
    );

    let opened = || open_wrap(&made, &reader.key).unwrap();
    assert_eq!(
        opened().verify(&cert(7)).unwrap_err(),
        HistoryError::NotSender
    );
    let forged = signed(6, "skald history", made.body_bytes());
    let forged = open_wrap(&forged, &reader.key).unwrap();
    assert_eq!(
        forged.verify(&cert(6)).unwrap_err(),
        HistoryError::Signature
    );
    assert_eq!(*opened().verify(&cert(6)).unwrap().to_bytes(), [3; 16]);

    assert_eq!(
        HistoryError::RecipientKey.to_string(),
        "the recipient's key is of small order"
    );
}
