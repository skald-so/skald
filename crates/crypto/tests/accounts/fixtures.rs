//! The tests' account, master keys and recovery keys; account bodies encoded with the tests' CBOR
//! writer; master backups sealed and opened with the `hpke` crate as `docs/protocol/membership.md`,
//! "Account", says; and a base32 decoder of RFC 4648, §6.

use skald_crypto::{HpkeKey, SigningKey};
use skald_proto::{AccountId, MasterBackup};

use crate::cbor::{Value, bytes};
pub use crate::master_backup::{BACKUP_INFO, base32};

/// The label of an account record's signature.
pub const LABEL: &str = "skald account";

/// The tests' account, and another one.
pub const ACCOUNT: AccountId = AccountId::from_bytes([0xac; 16]);
pub const OTHER_ACCOUNT: AccountId = AccountId::from_bytes([0xad; 16]);
/// The private keys of the account's master key and of another master key.
pub const MASTER: [u8; 32] = [0x4d; 32];
pub const OTHER_MASTER: [u8; 32] = [0x4e; 32];
/// The account's recovery key, and another one.
pub const RECOVERY: [u8; 32] = [0x7e; 32];
pub const OTHER_RECOVERY: [u8; 32] = [0x7f; 32];

/// The account's master key.
pub fn master() -> SigningKey {
    SigningKey::from_bytes(&MASTER)
}

/// The master public key of the private key `seed`.
pub fn public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).public_key()
}

/// The recovery HPKE public key of `recovery`: `DeriveKeyPair(recovery)`.
pub fn recovery_public(recovery: &[u8; 32]) -> [u8; 32] {
    HpkeKey::from_recovery_key(recovery).public_key()
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

/// The fields of the body of the tests' account record: its master key and recovery key, revision 1.
pub fn account_fields() -> Vec<(i64, Value)> {
    body_fields(ACCOUNT, &public(&MASTER), &recovery_public(&RECOVERY), 1)
}

/// `plaintext` sealed as a master backup to `recovery` on `account`, as "Account" says.
pub fn seal_as_specified(
    plaintext: &[u8],
    recovery: &[u8; 32],
    account: AccountId,
) -> MasterBackup {
    let (enc, ct) = crate::master_backup::seal(plaintext, recovery, account.as_bytes());
    MasterBackup { enc, ct }
}

/// What `enc` and `ct` open to as a master backup of `recovery` on `account`, if they open.
pub fn open_as_specified(
    enc: &[u8; 32],
    ct: &[u8],
    recovery: &[u8; 32],
    account: AccountId,
) -> Option<Vec<u8>> {
    crate::master_backup::open(enc, ct, recovery, account.as_bytes())
}

/// Whether `text` is in the shown form of a recovery key: 52 characters of the base32 alphabet of
/// RFC 4648, §6, in groups of four joined by `-`.
pub fn in_shown_form(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 13
        && groups.iter().all(|group| {
            group.len() == 4
                && group
                    .bytes()
                    .all(|c| c.is_ascii_uppercase() || (b'2'..=b'7').contains(&c))
        })
}
