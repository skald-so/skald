//! A channel's history under its history keys: `docs/protocol/membership.md`, "Key chain" and
//! "History entry". Links and entries are sealed with AES-128-GCM under an epoch's key, a random
//! nonce and the associated data the spec gives each.

use std::{cmp::Reverse, collections::BTreeMap, fmt};

use aes_gcm::{
    Aes128Gcm, KeyInit as _, Nonce,
    aead::{Aead as _, Payload},
};
use skald_proto::{
    Cbor, Cert, ChannelId, DeviceId, HISTORY_LABEL, HistoryBody, HistoryEntry, HistoryLink,
    HistoryPlaintext,
};
use zeroize::Zeroizing;

use crate::sign::{SigningKey, verify};

/// The history key of an epoch of a channel's group: `docs/protocol/membership.md`, "History
/// key". Its bytes are zeroed on drop and never shown.
pub struct HistoryKey(Zeroizing<[u8; 16]>);

impl HistoryKey {
    /// The key of these 16 bytes, such as those a wrap gives.
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// The key's 16 bytes.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 16]> {
        self.0.clone()
    }

    /// The link of `channel`'s epoch `epoch`, under this key, which is `epoch`'s: `prev_key`, the
    /// key of `prev`, the latest epoch before `epoch` the caller judged clean.
    ///
    /// # Errors
    /// [`HistoryError::Bounds`] unless `1 <= prev < epoch`.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn seal_link(
        &self,
        channel: ChannelId,
        epoch: u64,
        prev: u64,
        prev_key: &HistoryKey,
    ) -> Result<HistoryLink, HistoryError> {
        link_bounds(epoch, prev)?;
        let (nonce, ciphertext) = self.seal(
            &HistoryLink::associated_data(channel, epoch, prev),
            prev_key.0.as_slice(),
        );
        Ok(HistoryLink {
            channel,
            epoch,
            prev,
            nonce,
            ciphertext,
            unknown: Default::default(),
        })
    }

    /// The key of `link.prev`, from `link` under this key, which is `link.epoch`'s.
    ///
    /// # Errors
    /// [`HistoryError::Bounds`] unless `1 <= link.prev < link.epoch`; [`HistoryError::Open`] if it
    /// does not decrypt with this key; [`HistoryError::Malformed`] if it holds no 16-byte key.
    pub fn open_link(&self, link: &HistoryLink) -> Result<HistoryKey, HistoryError> {
        link_bounds(link.epoch, link.prev)?;
        let key = self.open(
            &HistoryLink::associated_data(link.channel, link.epoch, link.prev),
            &link.nonce,
            &link.ciphertext,
        )?;
        let key: &[u8; 16] = key
            .as_slice()
            .try_into()
            .map_err(|_| HistoryError::Malformed)?;
        Ok(Self::from_bytes(key))
    }

    /// The entry of `body` in its channel's epoch `epoch`, under this key, which is `epoch`'s:
    /// `body` signed by `sender`, the signing key of `body.sender`, over `["skald history", body]`.
    ///
    /// # Errors
    /// [`HistoryError::Bounds`] for epoch 0, which holds no messages; [`HistoryError::Malformed`]
    /// if `body` breaks its CDDL.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn seal_entry(
        &self,
        epoch: u64,
        body: &HistoryBody,
        sender: &SigningKey,
    ) -> Result<HistoryEntry, HistoryError> {
        if epoch == 0 {
            return Err(HistoryError::Bounds);
        }
        let bytes = body.to_cbor();
        let signature = sender.sign(HISTORY_LABEL, &[&bytes]);
        let plaintext = HistoryPlaintext::from_parts(bytes, signature)
            .map_err(|_| HistoryError::Malformed)?
            .to_cbor();
        let (nonce, ciphertext) = self.seal(
            &HistoryEntry::associated_data(body.channel, epoch),
            &plaintext,
        );
        Ok(HistoryEntry {
            channel: body.channel,
            epoch,
            nonce,
            ciphertext,
            unknown: Default::default(),
        })
    }

    /// The message `entry` holds, under this key, which is `entry.epoch`'s, once its body's
    /// `channel` is the entry's; [`HistoryMessage::verify`] checks its signature.
    ///
    /// # Errors
    /// [`HistoryError::Open`] if it does not decrypt with this key; [`HistoryError::Malformed`] if
    /// it holds no history plaintext; [`HistoryError::OtherChannel`] if the body is of another
    /// channel.
    pub fn open_entry(&self, entry: &HistoryEntry) -> Result<HistoryMessage, HistoryError> {
        let plaintext = self.open(
            &HistoryEntry::associated_data(entry.channel, entry.epoch),
            &entry.nonce,
            &entry.ciphertext,
        )?;
        let plaintext =
            HistoryPlaintext::from_cbor(&plaintext).map_err(|_| HistoryError::Malformed)?;
        if plaintext.body().channel != entry.channel {
            return Err(HistoryError::OtherChannel);
        }
        Ok(HistoryMessage(plaintext))
    }

    fn cipher(&self) -> Aes128Gcm {
        Aes128Gcm::new((&*self.0).into())
    }

    /// `plaintext` sealed under this key with a fresh random nonce and `aad`: the nonce and the
    /// ciphertext.
    fn seal(&self, aad: &[u8], plaintext: &[u8]) -> ([u8; 12], Vec<u8>) {
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).expect("the random generator failed");
        let ciphertext = self
            .cipher()
            .encrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .expect("a plaintext within AES-GCM's length bound");
        (nonce, ciphertext)
    }

    fn open(
        &self,
        aad: &[u8],
        nonce: &[u8; 12],
        ciphertext: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, HistoryError> {
        self.cipher()
            .decrypt(
                &Nonce::from(*nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| HistoryError::Open)
    }
}

impl fmt::Debug for HistoryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HistoryKey(..)")
    }
}

/// The links of the key chain hold the key of an earlier epoch from 1 on.
fn link_bounds(epoch: u64, prev: u64) -> Result<(), HistoryError> {
    if 1 <= prev && prev < epoch {
        Ok(())
    } else {
        Err(HistoryError::Bounds)
    }
}

/// The keys of the epochs of `channel` before `epoch` that `links` reach back from `key`, the key
/// of `epoch`, following every link it can open. Links of another channel and those that do not
/// open are skipped.
pub fn follow_links(
    channel: ChannelId,
    epoch: u64,
    key: &HistoryKey,
    links: &[HistoryLink],
) -> BTreeMap<u64, HistoryKey> {
    let mut links: Vec<&HistoryLink> = links
        .iter()
        .filter(|link| link.channel == channel)
        .collect();
    // A link's `prev` is below its epoch, so from the latest epoch down each link's key is known
    // by the time it comes up.
    links.sort_by_key(|link| Reverse(link.epoch));
    let mut reached = BTreeMap::new();
    for link in links {
        let held = if link.epoch == epoch {
            Some(key)
        } else {
            reached.get(&link.epoch)
        };
        if let Some(prev) = held.and_then(|held| held.open_link(link).ok()) {
            reached.entry(link.prev).or_insert(prev);
        }
    }
    reached
}

/// A history entry a reader decrypted, its signature not yet checked.
#[derive(Debug)]
pub struct HistoryMessage(HistoryPlaintext);

impl HistoryMessage {
    /// The device the body names as its sender.
    pub fn sender(&self) -> DeviceId {
        self.0.body().sender
    }

    /// The body, once its signature verifies with the signing key of `cert`, which the caller
    /// has checked ("History entry", check 2).
    ///
    /// # Errors
    /// [`HistoryError::NotSender`] if `cert` is not of the body's `sender`;
    /// [`HistoryError::Signature`] if the signature does not verify with its key.
    pub fn verify(self, cert: &Cert) -> Result<HistoryBody, HistoryError> {
        if cert.body().device != self.sender() {
            return Err(HistoryError::NotSender);
        }
        verify(
            &cert.body().signing,
            HISTORY_LABEL,
            &[self.0.body_bytes()],
            self.0.signature(),
        )
        .map_err(|_| HistoryError::Signature)?;
        Ok(self.0.body().clone())
    }
}

/// Why a history link or entry is not made or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryError {
    /// A link's epoch and `prev` are not `1 <= prev < epoch`, or an entry is of epoch 0.
    Bounds,
    /// It does not decrypt with the key.
    Open,
    /// It holds no 16-byte key or no history plaintext, or a body breaks its CDDL.
    Malformed,
    /// The body is of another channel than the entry.
    OtherChannel,
    /// The certificate is not of the body's sender.
    NotSender,
    /// The signature does not verify with the certificate's key.
    Signature,
}

impl fmt::Display for HistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bounds => "an epoch or prev outside the key chain's bounds",
            Self::Open => "it does not decrypt with the key",
            Self::Malformed => "it holds no history key or plaintext, or the body is malformed",
            Self::OtherChannel => "the body is of another channel than the entry",
            Self::NotSender => "the certificate is not the sender's",
            Self::Signature => "the signature does not verify with the sender's key",
        })
    }
}

impl std::error::Error for HistoryError {}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use skald_proto::{AccountId, Body, CertBody, Content, DeviceKind, MessageId, Unknown};

    use super::*;
    use crate::issue;

    const CHANNEL: ChannelId = ChannelId::from_bytes([1; 16]);

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn device_id(seed: u8) -> DeviceId {
        DeviceId::from_signing_key(&signing_key(seed).public_key())
    }

    fn cert(seed: u8) -> Cert {
        let body = CertBody {
            account: AccountId::from_bytes([9; 16]),
            device: device_id(seed),
            kind: DeviceKind::Client,
            signing: signing_key(seed).public_key(),
            hpke: [0; 32],
            issuer: None,
            issued: 0,
            ssh: None,
            name: "Test".to_owned(),
            unknown: Unknown::default(),
        };
        issue(&body, &signing_key(0), &signing_key(seed)).unwrap()
    }

    fn body(seed: u8, channel: ChannelId) -> HistoryBody {
        HistoryBody {
            sender: device_id(seed),
            channel,
            msg_id: MessageId::from_bytes([2; 16]),
            timestamp: 1,
            content: Content {
                from: "acme/alice/claude@skald.example".parse().unwrap(),
                to: None,
                counter: NonZeroU64::new(1),
                hops: 0,
                reply_to: None,
                body: Some(Body::A2a("{}".to_owned())),
                delegation: None,
                grant: None,
                attachments: None,
                unknown: Unknown::default(),
            },
            unknown: Unknown::default(),
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn each_refusal_gives_its_error() {
        let key = HistoryKey::from_bytes(&[3; 16]);
        let other = HistoryKey::from_bytes(&[4; 16]);

        let refused = key.seal_link(CHANNEL, 2, 2, &other);
        assert_eq!(refused.unwrap_err(), HistoryError::Bounds);
        let link = key.seal_link(CHANNEL, 2, 1, &other).unwrap();
        assert_eq!(other.open_link(&link).unwrap_err(), HistoryError::Open);
        let mut bounds = link.clone();
        bounds.prev = 0;
        assert_eq!(key.open_link(&bounds).unwrap_err(), HistoryError::Bounds);
        let (nonce, ciphertext) = key.seal(&HistoryLink::associated_data(CHANNEL, 2, 1), &[5; 15]);
        let short = HistoryLink {
            nonce,
            ciphertext,
            ..link
        };
        assert_eq!(key.open_link(&short).unwrap_err(), HistoryError::Malformed);

        let refused = key.seal_entry(0, &body(6, CHANNEL), &signing_key(6));
        assert_eq!(refused.unwrap_err(), HistoryError::Bounds);
        let entry = key
            .seal_entry(1, &body(6, CHANNEL), &signing_key(6))
            .unwrap();
        assert_eq!(other.open_entry(&entry).unwrap_err(), HistoryError::Open);
        let sealed = |plaintext: &[u8]| {
            let (nonce, ciphertext) =
                key.seal(&HistoryEntry::associated_data(CHANNEL, 1), plaintext);
            HistoryEntry {
                nonce,
                ciphertext,
                ..entry.clone()
            }
        };
        let malformed = sealed(&[0]);
        assert_eq!(
            key.open_entry(&malformed).unwrap_err(),
            HistoryError::Malformed
        );
        let foreign = body(6, ChannelId::from_bytes([7; 16])).to_cbor();
        let signature = signing_key(6).sign(HISTORY_LABEL, &[&foreign]);
        let plaintext = HistoryPlaintext::from_parts(foreign, signature).unwrap();
        let foreign = sealed(&plaintext.to_cbor());
        assert_eq!(
            key.open_entry(&foreign).unwrap_err(),
            HistoryError::OtherChannel
        );

        let message = || key.open_entry(&entry).unwrap();
        assert_eq!(
            message().verify(&cert(8)).unwrap_err(),
            HistoryError::NotSender
        );
        let forged = key
            .seal_entry(1, &body(6, CHANNEL), &signing_key(8))
            .unwrap();
        let forged = key.open_entry(&forged).unwrap();
        assert_eq!(
            forged.verify(&cert(6)).unwrap_err(),
            HistoryError::Signature
        );
        assert_eq!(message().verify(&cert(6)), Ok(body(6, CHANNEL)));
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn an_error_says_what_failed() {
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
                "the certificate is not the sender's",
            ),
            (
                HistoryError::Signature,
                "the signature does not verify with the sender's key",
            ),
        ] {
            assert_eq!(error.to_string(), message);
        }
    }
}
