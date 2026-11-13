//! A device's MLS state as the caller stores it: items, each sealed with XChaCha20-Poly1305 under
//! the device's state key, with a fresh random nonce and associated data that ties in a label, the
//! device ID and the item's name.

use std::fmt;

use chacha20poly1305::{
    KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use skald_proto::{DeviceId, signed_data};
use zeroize::Zeroizing;

use crate::sign::random_secret;

/// The label of a sealed item's associated data.
const LABEL: &str = "skald mls state";
/// The length of a nonce: XChaCha20-Poly1305's 24 bytes, safe to draw at random.
const NONCE_LEN: usize = 24;

/// The key that seals a device's MLS state; the caller keeps it in key storage with the device
/// keys. Its bytes are zeroed on drop and never shown.
pub struct StateKey(Zeroizing<[u8; 32]>);

impl StateKey {
    /// A new key from the cryptographically secure generator of the operating system, or in WASM
    /// of the browser or Node.js.
    ///
    /// # Panics
    /// If the random generator fails.
    pub fn generate() -> Self {
        Self(random_secret())
    }

    /// The key of these bytes, as [`StateKey::to_bytes`] gives them.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// The 32 bytes, for the device's key storage.
    pub fn to_bytes(&self) -> Zeroizing<[u8; 32]> {
        self.0.clone()
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new((&*self.0).into())
    }

    /// `plaintext` sealed as item `name` of `device`: the nonce, then the ciphertext.
    ///
    /// # Panics
    /// If the random generator fails.
    pub(crate) fn seal(&self, device: DeviceId, name: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0; NONCE_LEN];
        getrandom::fill(&mut nonce).expect("the random generator failed");
        let aad = associated_data(device, name);
        let ciphertext = self
            .cipher()
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .expect("an item within XChaCha20-Poly1305's length bound");
        [nonce.as_slice(), &ciphertext].concat()
    }

    /// The plaintext of item `name` of `device`, as [`StateKey::seal`] sealed it; `None` for
    /// anything else.
    pub(crate) fn open(
        &self,
        device: DeviceId,
        name: &[u8],
        sealed: &[u8],
    ) -> Option<Zeroizing<Vec<u8>>> {
        let (nonce, ciphertext) = sealed.split_at_checked(NONCE_LEN)?;
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;
        let aad = associated_data(device, name);
        self.cipher()
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: &aad,
                },
            )
            .ok()
            .map(Zeroizing::new)
    }
}

impl fmt::Debug for StateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StateKey(..)")
    }
}

/// The deterministic CBOR of `[LABEL, device, name]`.
fn associated_data(device: DeviceId, name: &[u8]) -> Vec<u8> {
    signed_data(LABEL, &[device.as_bytes(), name])
}

/// A device's MLS state, sealed: items, each a name the caller stores it under and its sealed
/// bytes. The caller saves the whole state after each step and keeps no older one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SealedState {
    items: Vec<(Vec<u8>, Vec<u8>)>,
}

impl SealedState {
    /// The state of these items, as the caller stored them.
    pub fn from_items(items: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    /// Each item: its name and its sealed bytes.
    pub fn items(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.items
            .iter()
            .map(|(name, sealed)| (name.as_slice(), sealed.as_slice()))
    }

    pub(crate) fn push(&mut self, name: Vec<u8>, sealed: Vec<u8>) {
        self.items.push((name, sealed));
    }
}
