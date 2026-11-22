//! The tests' account, master keys and recovery keys; account bodies encoded with the tests' CBOR
//! writer; and master backups sealed and opened as `docs/protocol/membership.md`, "Account", says,
//! with the `argon2`, `aes-gcm` and `hpke` crates.

use skald_crypto::{RecoveryKey, SigningKey};
use skald_proto::{AccountId, MasterBackup};

use crate::cbor::{Value, bytes};
use crate::master_backup::{derive, digits, open, recovery_public, seal};

/// The label of an account record's signature.
pub const LABEL: &str = "skald account";

/// The tests' account, and another one.
pub const ACCOUNT: AccountId = AccountId::from_bytes([0xac; 16]);
pub const OTHER_ACCOUNT: AccountId = AccountId::from_bytes([0xad; 16]);
/// The private keys of the account's master key and of another master key.
pub const MASTER: [u8; 32] = [0x4d; 32];
pub const OTHER_MASTER: [u8; 32] = [0x4e; 32];
/// The account's recovery key, and another one, in their shown form.
pub const RECOVERY: &str = "7EK4-QX2M-9PZD";
pub const OTHER_RECOVERY: &str = "7EK4-QX2M-9PZE";
/// A recovery HPKE public key, for the records' tests, which do not derive it.
pub const RECOVERY_PUBLIC: [u8; 32] = [0x7e; 32];

/// The account's master key.
pub fn master() -> SigningKey {
    SigningKey::from_bytes(&MASTER)
}

/// The master public key of the private key `seed`.
pub fn public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).public_key()
}

/// The recovery key `shown`, read by the code under test.
pub fn recovery_key(shown: &str) -> RecoveryKey {
    RecoveryKey::parse(shown).unwrap_or_else(|_| panic!("{shown:?} is not a recovery key"))
}

/// What the recovery key `shown` derives on `account`, as "Account" says: the backup's AEAD key and
/// the recovery HPKE public key.
pub fn derived(shown: &str, account: AccountId) -> ([u8; 32], [u8; 32]) {
    let digits = digits(shown).unwrap_or_else(|| panic!("{shown:?} is not in the shown form"));
    let (aead, ikm) = derive(&digits, account.as_bytes());
    (aead, recovery_public(&ikm))
}

/// The fields of an `account-body`.
pub fn body_fields(
    account: AccountId,
    master: &[u8; 32],
    recovery: &[u8; 32],
    revision: u64,
) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(account.as_bytes())),
        (2, bytes(master)),
        (3, bytes(recovery)),
        (4, Value::Uint(revision)),
    ]
}

/// The fields of the body of the tests' account record: its master key and [`RECOVERY_PUBLIC`],
/// revision 1.
pub fn account_fields() -> Vec<(i64, Value)> {
    body_fields(ACCOUNT, &public(&MASTER), &RECOVERY_PUBLIC, 1)
}

/// `plaintext` sealed as a master backup under `aead` on `account` with `nonce`, as "Account" says.
pub fn seal_as_specified(
    plaintext: &[u8],
    aead: &[u8; 32],
    account: AccountId,
    nonce: [u8; 12],
) -> Vec<u8> {
    seal(plaintext, aead, &nonce, account.as_bytes())
}

/// What `backup` opens to under `aead` on `account`, if it opens.
pub fn open_as_specified(
    backup: &MasterBackup,
    aead: &[u8; 32],
    account: AccountId,
) -> Option<Vec<u8>> {
    open(&backup.ct, aead, &backup.nonce, account.as_bytes())
}
