//! A device's X25519 key pair for HPKE: DHKEM(X25519, HKDF-SHA256) of `docs/protocol/README.md`,
//! "Scheme `hpke`".

use std::fmt;

use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::sign::random_secret;

/// An X25519 private key: a device's HPKE key. Its bytes are zeroed on drop and never shown.
pub struct HpkeKey(StaticSecret);

impl HpkeKey {
    /// The key of this 32-byte scalar, as RFC 7748 defines it: clamped only where it is used.
    pub fn from_bytes(secret: &[u8; 32]) -> Self {
        Self(StaticSecret::from(*secret))
    }

    /// A new key from the operating system's cryptographically secure generator.
    ///
    /// # Panics
    /// If the operating system's random generator fails.
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
}

impl fmt::Debug for HpkeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HpkeKey(..)")
    }
}
