//! Stateless HPKE: `docs/protocol/README.md`, "Scheme `hpke`". A device's X25519 key pair, the
//! single-shot seal and open in base mode of the suite, and the `hpke` payload on top of them.

use std::fmt;

use hpke::aead::AesGcm128;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable as _, Kem, OpModeR, OpModeS, Serializable as _};
use skald_proto::{
    Cbor, Cert, Content, DecodeError, DeviceId, Header, HpkePayload, HpkePlaintext, Scheme,
};
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::sign::{SigningKey, random_secret, verify};

/// `info` of an `hpke` payload, and the label of its signature.
const LABEL: &str = "skald hpke";

/// The KEM of the suite.
type Dhkem = X25519HkdfSha256;

/// An X25519 private key: a device's HPKE key. Its bytes are zeroed on drop and never shown.
pub struct HpkeKey(StaticSecret);

impl HpkeKey {
    /// The key of this 32-byte scalar, as RFC 7748 defines it: clamped only where it is used.
    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(StaticSecret::from(*secret))
    }

    /// The recovery HPKE key of an account's 32-byte `recovery` key: `DeriveKeyPair(recovery)` of
    /// DHKEM(X25519, HKDF-SHA256) (RFC 9180, §7.1.3), as `docs/protocol/membership.md`, "Account",
    /// says. Its [`HpkeKey::to_bytes`] are the derived private key, clamped.
    pub fn from_recovery_key(recovery: &[u8; 32]) -> Self {
        let (key, _) = <Dhkem as Kem>::derive_keypair(recovery);
        let mut secret = Zeroizing::new([0; 32]);
        key.write_exact(secret.as_mut());
        Self::from_bytes(&secret)
    }

    /// A new key from the cryptographically secure generator of the operating system, or in WASM
    /// of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        Self::from_bytes(&random_secret())
    }

    /// The scalar itself, as [`HpkeKey::from_bytes`] takes it, for the device's key storage.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The X25519 public key.
    pub fn public_key(&self) -> [u8; 32] {
        PublicKey::from(&self.0).to_bytes()
    }

    /// `plaintext` sealed to the X25519 key `recipient` in base mode, single-shot (RFC 9180,
    /// §6.1), with DHKEM(X25519, HKDF-SHA256), HKDF-SHA256 and AES-128-GCM: `enc` and `ct`. Each
    /// seal takes a new ephemeral key.
    ///
    /// # Errors
    /// [`HpkeError::RecipientKey`] if `recipient` is of small order.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn seal(
        recipient: &[u8; 32],
        info: &[u8],
        aad: &[u8],
        plaintext: &[u8],
    ) -> Result<([u8; 32], Vec<u8>), HpkeError> {
        let recipient = <Dhkem as Kem>::PublicKey::from_bytes(recipient)
            .map_err(|_| HpkeError::RecipientKey)?;
        let (enc, ct) = hpke::single_shot_seal::<AesGcm128, HkdfSha256, Dhkem>(
            &OpModeS::Base,
            &recipient,
            info,
            plaintext,
            aad,
        )
        .map_err(|_| HpkeError::RecipientKey)?;
        Ok((enc.to_bytes().into(), ct))
    }

    /// The plaintext that [`HpkeKey::seal`] sealed as `enc` and `ct` to this key's public key with
    /// `info` and `aad`.
    ///
    /// # Errors
    /// [`HpkeError::Open`] if they do not open with this key, `info` and `aad`.
    pub fn open(
        &self,
        enc: &[u8; 32],
        info: &[u8],
        aad: &[u8],
        ct: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, HpkeError> {
        let key = <Dhkem as Kem>::PrivateKey::from_bytes(self.to_bytes().as_ref())
            .map_err(|_| HpkeError::Open)?;
        let enc = <Dhkem as Kem>::EncappedKey::from_bytes(enc).map_err(|_| HpkeError::Open)?;
        hpke::single_shot_open::<AesGcm128, HkdfSha256, Dhkem>(
            &OpModeR::Base,
            &key,
            &enc,
            info,
            ct,
            aad,
        )
        .map(Zeroizing::new)
        .map_err(|_| HpkeError::Open)
    }
}

impl fmt::Debug for HpkeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HpkeKey(..)")
    }
}

/// The encoded `hpke-payload` of `content` under `header`: signed by the device of `sender`,
/// sealed to `recipient`, the HPKE key in the recipient device's certificate.
///
/// # Errors
/// [`HpkeError::Scheme`] or [`HpkeError::Recipients`] if `header` is not under `hpke` with one
/// device in `recipients`; [`HpkeError::RecipientKey`] if `recipient` is of small order.
///
/// # Panics
/// If the random generator fails.
pub fn seal_message(
    header: &Header,
    content: &Content,
    sender: &SigningKey,
    recipient: &[u8; 32],
) -> Result<Vec<u8>, HpkeError> {
    check_scheme(header)?;
    if header.recipients.as_ref().map(|devices| devices.len()) != Some(1) {
        return Err(HpkeError::Recipients);
    }
    let aad = header.to_cbor();
    let signature = sender.sign(LABEL, &[&aad, &content.to_cbor()]);
    let plaintext = Zeroizing::new(
        HpkePlaintext {
            sender: DeviceId::from_signing_key(&sender.public_key()),
            content: content.clone(),
            signature,
        }
        .to_cbor(),
    );
    let (enc, ct) = HpkeKey::seal(recipient, LABEL.as_bytes(), &aad, &plaintext)?;
    Ok(HpkePayload { enc, ct }.to_cbor())
}

/// Opens the `payload` of an envelope with `header` for the device `recipient` with its HPKE
/// `key`: check 1 of "Scheme `hpke`". Check 2 is [`HpkeMessage::verify`].
///
/// # Errors
/// [`HpkeError::Scheme`] if `header` is not under `hpke`; [`HpkeError::Recipients`] if
/// `recipients` is not `recipient` alone; [`HpkeError::Malformed`] if the payload or its plaintext
/// breaks the spec's CDDL; [`HpkeError::Open`] if it does not open with `key` under `header`.
pub fn open_message(
    header: &Header,
    payload: &[u8],
    recipient: DeviceId,
    key: &HpkeKey,
) -> Result<HpkeMessage, HpkeError> {
    check_scheme(header)?;
    if header.recipients.as_deref() != Some(&[recipient][..]) {
        return Err(HpkeError::Recipients);
    }
    let HpkePayload { enc, ct } = HpkePayload::from_cbor(payload).map_err(HpkeError::Malformed)?;
    let aad = header.to_cbor();
    let opened = key.open(&enc, LABEL.as_bytes(), &aad, &ct)?;
    let plaintext = HpkePlaintext::from_cbor(&opened).map_err(HpkeError::Malformed)?;
    Ok(HpkeMessage {
        header: aad,
        plaintext,
    })
}

fn check_scheme(header: &Header) -> Result<(), HpkeError> {
    match header.scheme {
        Scheme::Hpke => Ok(()),
        Scheme::Mls => Err(HpkeError::Scheme),
    }
}

/// An `hpke` payload that passed check 1. Its content is given once check 2 passes.
#[derive(Debug)]
pub struct HpkeMessage {
    /// The encoded header.
    header: Vec<u8>,
    plaintext: HpkePlaintext,
}

impl HpkeMessage {
    /// The device the payload names as `sender`: whose certificate [`HpkeMessage::verify`] takes.
    pub fn sender(&self) -> DeviceId {
        self.plaintext.sender
    }

    /// The content, once `signature` verifies with the signing key of `cert`, the valid
    /// certificate of `sender` the caller has checked: check 2 of "Scheme `hpke`".
    ///
    /// # Errors
    /// [`HpkeError::Certificate`] if `cert` is not `sender`'s; [`HpkeError::Signature`] if
    /// `signature` does not verify with its key.
    pub fn verify(self, cert: &Cert) -> Result<Content, HpkeError> {
        let body = cert.body();
        if body.device != self.plaintext.sender {
            return Err(HpkeError::Certificate);
        }
        let content = self.plaintext.content;
        verify(
            &body.signing,
            LABEL,
            &[&self.header, &content.to_cbor()],
            &self.plaintext.signature,
        )
        .map_err(|_| HpkeError::Signature)?;
        Ok(content)
    }
}

/// Why an `hpke` seal or open fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HpkeError {
    /// The recipient's X25519 key is of small order, so the shared secret is all zeros.
    RecipientKey,
    /// The header is not under `hpke`.
    Scheme,
    /// Check 1: `recipients` is not one device, or not the opening one.
    Recipients,
    /// The payload or its plaintext breaks the spec's CDDL.
    Malformed(DecodeError),
    /// Check 1: it does not open with the key, `info` and `aad`.
    Open,
    /// Check 2: the certificate is not `sender`'s.
    Certificate,
    /// Check 2: `signature` does not verify with the key of `sender`'s certificate.
    Signature,
}

impl fmt::Display for HpkeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecipientKey => f.write_str("the recipient's key is of small order"),
            Self::Scheme => f.write_str("the header is not under hpke"),
            Self::Recipients => f.write_str("recipients is not the one opening device"),
            Self::Malformed(error) => write!(f, "a malformed hpke payload: {error}"),
            Self::Open => f.write_str("the payload does not open with the key"),
            Self::Certificate => f.write_str("the certificate is not the sender's"),
            Self::Signature => f.write_str("the signature does not verify with the sender's key"),
        }
    }
}

impl std::error::Error for HpkeError {}
