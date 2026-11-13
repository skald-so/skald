//! Ed25519 signatures over a label and byte strings: `docs/protocol/README.md`, "Conventions".

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, VerifyingKey};
use skald_proto::signed_data;
use zeroize::Zeroizing;

/// An Ed25519 private key: a device's signing key or an account's master key. Its bytes are
/// zeroed on drop and never shown.
pub struct SigningKey(ed25519_dalek::SigningKey);

impl SigningKey {
    /// The key of this 32-byte secret, as RFC 8032 defines it.
    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(secret))
    }

    /// A new key from the cryptographically secure generator of the operating system, or in WASM
    /// of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        Self::from_bytes(&random_secret())
    }

    /// The 32-byte secret, as [`SigningKey::from_bytes`] takes it, for the device's key storage.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }

    /// The Ed25519 public key.
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }

    /// The signature over the encoded `[label, parts...]`, the parts as byte strings.
    pub fn sign(&self, label: &str, parts: &[&[u8]]) -> [u8; 64] {
        self.0.sign(&signed_data(label, parts)).to_bytes()
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SigningKey(..)")
    }
}

/// 32 bytes from the cryptographically secure generator of the operating system, or in WASM of the
/// browser or Node.js, zeroed on drop.
///
/// # Panics
/// If the random generator fails.
pub(crate) fn random_secret() -> Zeroizing<[u8; 32]> {
    let mut secret = Zeroizing::new([0; 32]);
    getrandom::fill(secret.as_mut()).expect("the random generator failed");
    secret
}

/// A signature does not verify.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureError;

impl fmt::Display for SignatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the signature does not verify")
    }
}

impl std::error::Error for SignatureError {}

/// Checks `signature` over the encoded `[label, parts...]` with `public_key`. Ed25519 as
/// ed25519-dalek's `verify_strict` checks it: a public key or signature point of small order
/// fails.
///
/// # Errors
/// [`SignatureError`] if `public_key` is not a valid key or `signature` does not verify with it.
pub fn verify(
    public_key: &[u8; 32],
    label: &str,
    parts: &[&[u8]],
    signature: &[u8; 64],
) -> Result<(), SignatureError> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| SignatureError)?;
    key.verify_strict(
        &signed_data(label, parts),
        &Signature::from_bytes(signature),
    )
    .map_err(|_| SignatureError)
}
