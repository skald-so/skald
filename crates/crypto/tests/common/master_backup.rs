//! The recovery key's derivation and the master backup as `docs/protocol/membership.md`, "Account",
//! says, with the `argon2`, `aes-gcm` and `hpke` crates apart from the code under test, and the
//! recovery key's shown form: for the tests of `skald-crypto` and of the CLI.

use aes_gcm::aead::{Aead as _, Payload};
use aes_gcm::{Aes256Gcm, KeyInit as _, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use hpke::kem::X25519HkdfSha256;
use hpke::{Kem as _, Serializable as _};

/// Crockford's base32 alphabet, in the order of its values.
pub const ALPHABET: &str = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Argon2id's memory in KiB, passes and lanes: the spec's, and a test build's of skald-crypto with
/// the feature `test-cost`.
const SPEC_COST: (u32, u32, u32) = (262_144, 8, 1);
const TEST_COST: (u32, u32, u32) = (8, 1, 1);
/// What comes before the account ID in the salt.
const SALT_PREFIX: &[u8] = b"skald recovery";

/// The 12 characters of `shown`, a key in its shown form: 3 groups of 4 characters of [`ALPHABET`]
/// joined by `-`; `None` for any other text.
pub fn digits(shown: &str) -> Option<[u8; 12]> {
    let shaped = shown.len() == 14
        && shown.bytes().enumerate().all(|(at, c)| match at {
            4 | 9 => c == b'-',
            _ => ALPHABET.as_bytes().contains(&c),
        });
    shaped.then(|| {
        shown
            .replace('-', "")
            .into_bytes()
            .try_into()
            .expect("12 characters")
    })
}

/// The shown form of the key of `digits`: 3 groups of 4 joined by `-`.
pub fn shown_form(digits: &[u8; 12]) -> String {
    let groups: Vec<&str> = digits
        .chunks(4)
        .map(|group| std::str::from_utf8(group).expect("the alphabet is ASCII"))
        .collect();
    groups.join("-")
}

/// Argon2id of the key of `digits` on `account`, its 16 bytes, at the spec's cost: the backup's
/// AES-256-GCM key and the IKM of the recovery HPKE key pair.
pub fn derive(digits: &[u8; 12], account: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    derive_at(SPEC_COST, digits, account)
}

/// [`derive`] at the cost the code under test derives at in this process: the spec's, or a test
/// build's, as `skald_crypto::derives_at_spec_cost` says. For the tests of the CLI, whose `skald`
/// is a test build.
pub fn derive_as_built(digits: &[u8; 12], account: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
    let cost = if skald_crypto::derives_at_spec_cost() {
        SPEC_COST
    } else {
        TEST_COST
    };
    derive_at(cost, digits, account)
}

fn derive_at(
    (memory, passes, lanes): (u32, u32, u32),
    digits: &[u8; 12],
    account: &[u8; 16],
) -> ([u8; 32], [u8; 32]) {
    let params = Params::new(memory, passes, lanes, Some(64)).expect("Argon2id parameters");
    let salt = [SALT_PREFIX, account].concat();
    let mut out = [0; 64];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(digits, &salt, &mut out)
        .expect("Argon2id");
    let (aead, ikm) = out.split_at(32);
    (
        aead.try_into().expect("32 bytes"),
        ikm.try_into().expect("32 bytes"),
    )
}

/// The recovery HPKE public key of `ikm`: `DeriveKeyPair(ikm)` of DHKEM(X25519, HKDF-SHA256).
pub fn recovery_public(ikm: &[u8; 32]) -> [u8; 32] {
    let (_, public) = X25519HkdfSha256::derive_keypair(ikm);
    public.to_bytes().into()
}

/// `plaintext` sealed under `aead` with `nonce`, the account's 16 bytes as associated data.
pub fn seal(plaintext: &[u8], aead: &[u8; 32], nonce: &[u8; 12], account: &[u8; 16]) -> Vec<u8> {
    Aes256Gcm::new(aead.into())
        .encrypt(
            &Nonce::from(*nonce),
            Payload {
                msg: plaintext,
                aad: account,
            },
        )
        .expect("a seal")
}

/// What `ct` opens to under `aead` with `nonce` on `account`, if it opens.
pub fn open(ct: &[u8], aead: &[u8; 32], nonce: &[u8; 12], account: &[u8; 16]) -> Option<Vec<u8>> {
    Aes256Gcm::new(aead.into())
        .decrypt(
            &Nonce::from(*nonce),
            Payload {
                msg: ct,
                aad: account,
            },
        )
        .ok()
}
