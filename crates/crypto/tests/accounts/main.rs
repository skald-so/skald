//! Account records and master backups, natively and in WASM: `docs/protocol/membership.md`,
//! "Account". The types of `skald-proto` keep their bytes as "Signed objects" says; `skald-crypto`
//! signs and checks a record with the master key over its label, derives from the recovery key on
//! the account with Argon2id the backup's AEAD key and the recovery HPKE key pair, seals and opens
//! the master backup with AES-256-GCM, and refuses an opened key that does not match `master`. The
//! recovery key's form: `recovery_key_form.rs`. The vectors of `docs/protocol/test-vectors.md`,
//! "Account record" and "Master backup", are checked against both.
//!
//! The API, as these tests use it:
//! - `skald_proto::{ACCOUNT_LABEL, AccountBody, AccountRecord, MasterBackup}`: `AccountRecord` is
//!   `Signed<AccountBody>`; `AccountBody { account, master, recovery, revision, unknown }`;
//!   `MasterBackup { nonce: [u8; 12], ct: [u8; 48] }`, each `Cbor`;
//! - `skald_crypto::sign_account(&AccountBody, &SigningKey) -> Result<AccountRecord, _>` and
//!   `check_account(&AccountRecord, master: &[u8; 32]) -> Result<_, _>`;
//! - `skald_crypto::new_recovery(&SigningKey, &RecoveryKey, AccountId) -> (AccountBody,
//!   MasterBackup)` and `new_recovery_with_nonce(.., nonce: [u8; 12])`, for the vector;
//!   `open_recovery(&AccountRecord, &MasterBackup, &RecoveryKey) -> Result<Recovered,
//!   AccountError>`.
//!
//! The fixtures encode records with the tests' CBOR writer, sign them with the tests' keys and
//! derive keys and seal and open backups with the `argon2`, `aes-gcm` and `hpke` crates as
//! "Account" says, apart from the code under test.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
#[path = "../common/fields.rs"]
mod fields;
#[allow(dead_code)]
#[path = "../common/master_backup.rs"]
mod master_backup;

mod backups;
mod fixtures;
mod records;
mod vectors;
