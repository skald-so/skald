//! The invite's secret and the invite key derived from it (`membership.md`, "Invite", "Secret"):
//! the Ed25519 private key `HKDF-SHA256(salt = "", IKM = secret, info = "skald invite", L = 32)`.

use std::collections::HashSet;

use skald_crypto::{InviteSecret, verify};

use crate::common::hex32;
use crate::objects::{OTHER_SECRET, SECRET, invite_dalek, invite_public};

/// The secret of the bytes 00 to 1f.
fn counting() -> [u8; 32] {
    std::array::from_fn(|at| u8::try_from(at).unwrap())
}

/// The invite private key of [`counting`], computed apart from the crates with Python's `hmac`.
const COUNTING_PRIVATE: &str = "22a87520d8720b891e2ea1201d017e0022be6bb4cf015b5387291038819ce64b";

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_invite_key_is_hkdf_sha256_of_the_secret() {
    let secret = InviteSecret::from_bytes(&counting());
    let key = secret.invite_key();
    assert_eq!(*key.to_bytes(), hex32(COUNTING_PRIVATE), "the private key");
    assert_eq!(
        key.public_key(),
        ed25519_dalek::SigningKey::from_bytes(&hex32(COUNTING_PRIVATE))
            .verifying_key()
            .to_bytes(),
        "the public key"
    );
    for bytes in [SECRET, OTHER_SECRET, [0; 32], [0xff; 32], counting()] {
        let key = InviteSecret::from_bytes(&bytes).invite_key();
        assert_eq!(
            *key.to_bytes(),
            invite_dalek(&bytes).to_bytes(),
            "{bytes:02x?}"
        );
        assert_eq!(key.public_key(), invite_public(&bytes), "{bytes:02x?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_secret_keeps_its_bytes_and_derives_the_same_key_each_time() {
    let secret = InviteSecret::from_bytes(&SECRET);
    assert_eq!(*secret.to_bytes(), SECRET);
    assert_eq!(
        secret.invite_key().public_key(),
        secret.invite_key().public_key()
    );
    assert_ne!(
        secret.invite_key().public_key(),
        InviteSecret::from_bytes(&OTHER_SECRET)
            .invite_key()
            .public_key()
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn generated_secrets_differ_and_derive_their_keys() {
    let secrets: Vec<InviteSecret> = (0..16).map(|_| InviteSecret::generate()).collect();
    let bytes: HashSet<[u8; 32]> = secrets.iter().map(|secret| *secret.to_bytes()).collect();
    assert_eq!(bytes.len(), secrets.len(), "no two secrets are the same");
    assert!(!bytes.contains(&[0; 32]), "no secret is all zeros");
    let keys: HashSet<[u8; 32]> = secrets
        .iter()
        .map(|secret| secret.invite_key().public_key())
        .collect();
    assert_eq!(keys.len(), secrets.len(), "no two invite keys are the same");
    for secret in &secrets {
        assert_eq!(
            secret.invite_key().public_key(),
            invite_public(&secret.to_bytes())
        );
    }

    // The invite key signs, and its signature verifies with its public key only.
    let signature = secrets[0].invite_key().sign("skald test", &[b"part"]);
    let public = secrets[0].invite_key().public_key();
    assert_eq!(
        verify(&public, "skald test", &[b"part"], &signature),
        Ok(())
    );
    let other = secrets[1].invite_key().public_key();
    assert!(verify(&other, "skald test", &[b"part"], &signature).is_err());
}
