//! Account records, master backups and recovery keys, natively and in WASM:
//! `docs/protocol/membership.md`, "Account". The types of `skald-proto` keep their bytes as "Signed
//! objects" says; `skald-crypto` signs and checks a record with the master key over its label, makes
//! the recovery key, its shown form and its input, and seals and opens the master backup in HPKE PSK
//! mode, refusing an opened key that does not match `master`. The vectors of
//! `docs/protocol/test-vectors.md`, "Account record" and "Master backup", are checked against both.
//!
//! The API, as these tests use it:
//! - `skald_proto::{ACCOUNT_LABEL, AccountBody, AccountRecord, MasterBackup}`: `AccountRecord` is
//!   `Signed<AccountBody>`; `AccountBody { account, master, recovery, revision, unknown }`;
//!   `MasterBackup { enc: [u8; 32], ct: Vec<u8> }`, each `Cbor`;
//! - `skald_crypto::sign_account(&AccountBody, &SigningKey) -> Result<AccountRecord, _>` and
//!   `check_account(&AccountRecord, master: &[u8; 32]) -> Result<_, _>`;
//! - `skald_crypto::RecoveryKey`: `generate()`, `from_bytes(&[u8; 32])`, `to_bytes()`, `shown()`, the
//!   text a person writes down, and `parse(&str) -> Result<RecoveryKey, _>`, the text a person types;
//! - `skald_crypto::seal_master_backup(&SigningKey, &RecoveryKey, AccountId) -> MasterBackup`;
//!   `seal_master_backup_with_ephemeral(.., ephemeral: &[u8; 32])`, whose ephemeral key pair is
//!   `DeriveKeyPair(ephemeral)`, for the vector; `open_master_backup(&MasterBackup, &RecoveryKey,
//!   AccountId, master: &[u8; 32]) -> Result<SigningKey, _>`.
//!
//! The fixtures encode records with the tests' CBOR writer, sign them with the tests' keys and seal
//! and open backups with the `hpke` crate as "Account" says, apart from the code under test.

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
mod keys;
mod records;
mod vectors;
