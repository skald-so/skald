//! A device's Ed25519 signing key, generated outside the MLS library, and its device ID.

use ed25519_dalek::{Signer as _, SigningKey};
use sha2::{Digest, Sha256};

/// A 16-byte device ID: `docs/protocol/README.md`, "Identifiers".
pub type DeviceId = [u8; 16];

/// A device's signing key, as the device keeps it in its key store.
#[derive(Clone)]
pub struct Identity {
    signing: SigningKey,
}

impl Identity {
    /// A new random signing key.
    pub fn generate() -> Self {
        let mut secret = [0; 32];
        getrandom::fill(&mut secret).expect("the OS random number generator works");
        Self {
            signing: SigningKey::from_bytes(&secret),
        }
    }

    /// The device ID derived from the public signing key.
    pub fn device_id(&self) -> DeviceId {
        device_id(&self.public_key())
    }

    /// The Ed25519 public key: the leaf's `signature_key`.
    pub fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// The 32-byte Ed25519 secret key (the seed of RFC 8032).
    pub fn secret_key(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    /// An Ed25519 signature of `message`.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }
}

/// The device ID of an Ed25519 public signing key: a UUIDv8 of the first 16 bytes of SHA-256 over
/// the deterministic CBOR `["skald device", signing]`.
pub fn device_id(signing: &[u8; 32]) -> DeviceId {
    // Array of 2; text string of 12 bytes; byte string of 32 bytes.
    let mut encoded = vec![0x82, 0x6c];
    encoded.extend_from_slice(b"skald device");
    encoded.extend_from_slice(&[0x58, 0x20]);
    encoded.extend_from_slice(signing);
    let hash = Sha256::digest(&encoded);
    let mut id: DeviceId = hash[..16].try_into().expect("SHA-256 has 32 bytes");
    id[6] = (id[6] & 0x0f) | 0x80;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_is_a_uuidv8_of_the_key_hash() {
        let key = [7; 32];
        let id = device_id(&key);
        assert_eq!(id[6] >> 4, 8, "version");
        assert_eq!(id[8] >> 6, 0b10, "variant");

        let mut encoded = b"\x82\x6cskald device\x58\x20".to_vec();
        encoded.extend_from_slice(&key);
        let hash = Sha256::digest(&encoded);
        for (i, (got, want)) in id.iter().zip(&hash[..16]).enumerate() {
            let mask = match i {
                6 => 0x0f,
                8 => 0x3f,
                _ => 0xff,
            };
            assert_eq!(got & mask, want & mask, "byte {i}");
        }
    }

    #[test]
    fn different_keys_have_different_ids() {
        let (a, b) = (Identity::generate(), Identity::generate());
        assert_ne!(a.public_key(), b.public_key());
        assert_ne!(a.device_id(), b.device_id());
        assert_eq!(a.device_id(), device_id(&a.public_key()));
    }

    #[test]
    fn signatures_verify_with_the_public_key() {
        let identity = Identity::generate();
        let key = ed25519_dalek::VerifyingKey::from_bytes(&identity.public_key()).unwrap();
        let signature = ed25519_dalek::Signature::from_bytes(&identity.sign(b"message"));
        assert!(key.verify_strict(b"message", &signature).is_ok());
        assert!(key.verify_strict(b"other", &signature).is_err());
    }
}
