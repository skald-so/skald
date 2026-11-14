//! Wraps of an epoch's history key to a browser or an account's recovery key:
//! `docs/protocol/membership.md`, "Wraps". The key is sealed with the single-shot HPKE of
//! [`HpkeKey::seal`], `info` `skald history wrap`, `aad` the encoded `[channel, epoch, recipient]`,
//! and the wrap is signed by a member device of the epoch.

use skald_proto::{
    Cbor, Cert, ChannelId, DeviceId, HistoryWrap, HistoryWrapBody, SignedBody, WrapRecipient,
};

use crate::history::{HistoryError, HistoryKey, verify_signed};
use crate::hpke::HpkeKey;
use crate::sign::SigningKey;

/// `info` of a wrap's seal.
const INFO: &[u8] = b"skald history wrap";
/// The label of a wrap's signature.
const LABEL: &str = <HistoryWrapBody as SignedBody>::LABEL;

impl HistoryKey {
    /// The wrap of this key, the key of `channel`'s epoch `epoch`, to `recipient`: sealed to
    /// `recipient_key`, the `hpke` key of a browser's certificate or the recovery HPKE key of an
    /// account's record, and signed by `signer`, the signing key of a member device of the epoch.
    ///
    /// # Errors
    /// [`HistoryError::Bounds`] for epoch 0; [`HistoryError::RecipientKey`] if `recipient_key` is
    /// of small order.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn wrap(
        &self,
        channel: ChannelId,
        epoch: u64,
        recipient: WrapRecipient,
        recipient_key: &[u8; 32],
        signer: &SigningKey,
    ) -> Result<HistoryWrap, HistoryError> {
        if epoch == 0 {
            return Err(HistoryError::Bounds);
        }
        let (enc, ct) = HpkeKey::seal(
            recipient_key,
            INFO,
            &HistoryWrapBody::associated_data(channel, epoch, &recipient),
            self.to_bytes().as_slice(),
        )
        .map_err(|_| HistoryError::RecipientKey)?;
        let body = HistoryWrapBody {
            channel,
            epoch,
            recipient,
            signer: DeviceId::from_signing_key(&signer.public_key()),
            enc,
            ct,
            unknown: Default::default(),
        }
        .to_cbor();
        let signature = signer.sign(LABEL, &[&body]);
        Ok(HistoryWrap::from_parts(body, signature).expect("a wrap body of an epoch from 1 on"))
    }
}

/// The history key `wrap` holds, opened with `key`, the HPKE key of its recipient;
/// [`OpenedWrap::verify`] checks its signature.
///
/// # Errors
/// [`HistoryError::Open`] if `ct` does not open with `key` under the wrap's channel, epoch and
/// recipient; [`HistoryError::Malformed`] if it holds no 16-byte key.
pub fn open_wrap(wrap: &HistoryWrap, key: &HpkeKey) -> Result<OpenedWrap, HistoryError> {
    let body = wrap.body();
    let opened = key
        .open(
            &body.enc,
            INFO,
            &HistoryWrapBody::associated_data(body.channel, body.epoch, &body.recipient),
            &body.ct,
        )
        .map_err(|_| HistoryError::Open)?;
    Ok(OpenedWrap {
        wrap: wrap.clone(),
        key: HistoryKey::from_opened(&opened)?,
    })
}

/// A wrap a reader opened, its signature not yet checked.
#[derive(Debug)]
pub struct OpenedWrap {
    wrap: HistoryWrap,
    key: HistoryKey,
}

impl OpenedWrap {
    /// The device the wrap names as `signer`: whose certificate [`OpenedWrap::verify`] takes.
    pub fn signer(&self) -> DeviceId {
        self.wrap.body().signer
    }

    /// The epoch's key, once the signature verifies with the signing key of `cert`, the
    /// certificate of `signer` the caller has checked.
    ///
    /// # Errors
    /// [`HistoryError::NotSender`] if `cert` is not `signer`'s; [`HistoryError::Signature`] if the
    /// signature does not verify with its key.
    pub fn verify(self, cert: &Cert) -> Result<HistoryKey, HistoryError> {
        verify_signed(
            cert,
            self.signer(),
            LABEL,
            self.wrap.body_bytes(),
            self.wrap.signature(),
        )?;
        Ok(self.key)
    }
}
