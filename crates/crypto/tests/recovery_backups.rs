//! The account record and the master backup of a recovery key of 12 characters, natively and in
//! WASM (`docs/protocol/membership.md`, "Account"): Argon2id with the spec's parameters and a
//! per-account salt turns the recovery key into the AEAD key that seals the master backup and into
//! the recovery HPKE key pair, whose public key the account record names. A backup opens only with
//! its recovery key and its account's salt, and an opened key that does not match the record's
//! `master` is refused. The vectors of `docs/protocol/test-vectors.md`, "Account record" and
//! "Master backup", are those of [`VECTOR_KEY`].
//!
//! The API, as these tests use it, beside that of the account record (`sign_account`,
//! `check_account`) and of the recovery key's form (`recovery_key_form.rs`):
//! - `skald_crypto::new_recovery(&SigningKey, &RecoveryKey, AccountId) -> (AccountBody,
//!   MasterBackup)`: the master key's backup sealed with the recovery key on the account, and the
//!   body of its account record at revision 1, which names the account, the master public key and
//!   the recovery HPKE public key; one derivation;
//! - `skald_crypto::open_recovery(&AccountRecord, &MasterBackup, &RecoveryKey) -> Result<Recovered,
//!   AccountError>`: the backup opened with the recovery key, refused unless it opens to the private
//!   key of the record's `master`; one derivation. `Recovered { master: SigningKey, hpke: HpkeKey }`
//!   holds the master key and the recovery HPKE key pair;
//! - `skald_crypto::derive_at_spec_cost()`: the derivations at the spec's cost in a test build too,
//!   as each test here derives.
//!
//! Where the salt is kept is the spec's: these tests take it only through these calls.

#[allow(dead_code)]
mod common;

use skald_crypto::{
    RecoveryKey, SigningKey, check_account, derive_at_spec_cost, new_recovery, open_recovery,
    sign_account,
};
use skald_proto::{AccountId, AccountRecord, Cbor, MasterBackup};

use common::{hex, hex32};

/// The test vectors.
const DOC: &str = include_str!("../../../docs/protocol/test-vectors.md");

/// The recovery key of the vectors, in its shown form.
const VECTOR_KEY: &str = "0123-4567-89AB";

/// Alice's account, that of the other vectors, and another account.
const ACCOUNT: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9";
const OTHER_ACCOUNT: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5ea";

/// RFC 8032, §7.1, "TEST 1": the secret key and its public key, the vectors' master key.
const MASTER_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const MASTER_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn account(id: &str) -> AccountId {
    id.parse().expect("an account ID")
}

fn key(shown: &str) -> RecoveryKey {
    RecoveryKey::parse(shown).unwrap_or_else(|_| panic!("{shown:?} is not a recovery key"))
}

/// The account record and the master backup of `master` and `recovery` on `account`, the record
/// signed by `master`.
fn made(
    master: &SigningKey,
    recovery: &RecoveryKey,
    account: AccountId,
) -> (AccountRecord, MasterBackup) {
    let (body, backup) = new_recovery(master, recovery, account);
    let record = sign_account(&body, master).expect("the record's body is valid");
    (record, backup)
}

/// The bytes of the `hex` block of the section `## <name>` of the test vectors.
fn vector(name: &str) -> Vec<u8> {
    let heading = format!("## {name}");
    let mut section = DOC
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .skip_while(|line| *line != heading)
        .skip(1)
        .take_while(|line| !line.starts_with("## "));
    let digits: String = section
        .by_ref()
        .skip_while(|line| *line != "```hex")
        .skip(1)
        .take_while(|line| *line != "```")
        .collect();
    assert!(!digits.is_empty(), "test-vectors.md has no hex of {name:?}");
    hex(&digits)
}

/// A new recovery gives the record of the account and its master key at revision 1, which the
/// master key signs. The record and the backup keep their bytes through CBOR, and, decoded, open
/// with the recovery key, read back from its shown form, to the master private key and to the
/// recovery HPKE key pair the record names.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_and_record_open_with_their_recovery_key_to_the_master_key_and_the_records_hpke_key() {
    derive_at_spec_cost();
    let master = SigningKey::generate();
    let recovery = RecoveryKey::generate();
    let on = account(ACCOUNT);

    let (record, backup) = made(&master, &recovery, on);

    let body = record.body();
    assert_eq!(body.account, on);
    assert_eq!(body.master, master.public_key());
    assert_eq!(body.revision, 1);
    check_account(&record, &master.public_key()).expect("the master key signs the record");
    let record_bytes = record.to_cbor();
    let backup_bytes = backup.to_cbor();
    let record = AccountRecord::from_cbor(&record_bytes).expect("the record decodes");
    let backup = MasterBackup::from_cbor(&backup_bytes).expect("the backup decodes");
    assert_eq!(record.to_cbor(), record_bytes, "the record, byte for byte");
    assert_eq!(backup.to_cbor(), backup_bytes, "the backup, byte for byte");
    let typed = key(recovery.shown().as_str());
    let recovered =
        open_recovery(&record, &backup, &typed).expect("the backup opens with its recovery key");
    assert_eq!(*recovered.master.to_bytes(), *master.to_bytes());
    assert_eq!(
        recovered.hpke.public_key(),
        record.body().recovery,
        "the derived recovery HPKE key is the record's"
    );
}

/// The backup does not open with another key, also one a character away from its own.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_opens_only_with_its_recovery_key() {
    derive_at_spec_cost();
    let master = SigningKey::generate();
    let (record, backup) = made(&master, &key(VECTOR_KEY), account(ACCOUNT));

    for wrong in ["0123-4567-89AC", "1123-4567-89AB"] {
        assert!(
            open_recovery(&record, &backup, &key(wrong)).is_err(),
            "the backup of {VECTOR_KEY} opens with {wrong}"
        );
    }
}

/// One recovery key on two accounts gives two recovery HPKE keys, and the backup of one account
/// opens with neither the other's record nor its own record with the other's backup: the salt is
/// the account's own.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_derivation_is_salted_per_account() {
    derive_at_spec_cost();
    let master = SigningKey::generate();
    let recovery = key(VECTOR_KEY);

    let (first_record, first_backup) = made(&master, &recovery, account(ACCOUNT));
    let (other_record, other_backup) = made(&master, &recovery, account(OTHER_ACCOUNT));

    assert_ne!(
        first_record.body().recovery,
        other_record.body().recovery,
        "one recovery HPKE key on two accounts"
    );
    assert!(
        open_recovery(&first_record, &other_backup, &recovery).is_err(),
        "another account's backup opens"
    );
    assert!(
        open_recovery(&other_record, &first_backup, &recovery).is_err(),
        "a backup opens with another account's record"
    );
}

/// A record that names another master key, signed by it, with the record's other fields as they
/// were, does not open the backup.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_whose_key_is_not_the_records_master_key_is_refused() {
    derive_at_spec_cost();
    let master = SigningKey::generate();
    let recovery = RecoveryKey::generate();
    let (record, backup) = made(&master, &recovery, account(ACCOUNT));
    let other = SigningKey::generate();
    let mut body = record.body().clone();
    body.master = other.public_key();
    let forged = sign_account(&body, &other).expect("the record's body is valid");

    assert!(
        open_recovery(&forged, &backup, &recovery).is_err(),
        "the backup opens to a key the record does not name"
    );
}

/// The vectors: Alice's account record, revision 1, names the master key "TEST 1", which signs it,
/// and the master backup; both decode and encode back to their bytes, and the backup opens with
/// [`VECTOR_KEY`] to the private key "TEST 1" and to the recovery HPKE key the record names, and
/// not with a key a character away.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_are_the_record_and_backup_of_their_recovery_key() {
    derive_at_spec_cost();
    let record_bytes = vector("Account record");
    let backup_bytes = vector("Master backup");

    let record = AccountRecord::from_cbor(&record_bytes).expect("the record vector decodes");
    let backup = MasterBackup::from_cbor(&backup_bytes).expect("the backup vector decodes");

    assert_eq!(record.to_cbor(), record_bytes, "the record, byte for byte");
    assert_eq!(backup.to_cbor(), backup_bytes, "the backup, byte for byte");
    let body = record.body();
    assert_eq!(body.account, account(ACCOUNT));
    assert_eq!(body.master, hex32(MASTER_PUBLIC));
    assert_eq!(body.revision, 1);
    check_account(&record, &hex32(MASTER_PUBLIC)).expect("\"TEST 1\" signs the record");
    let recovered = open_recovery(&record, &backup, &key(VECTOR_KEY))
        .expect("the backup vector opens with its recovery key");
    assert_eq!(*recovered.master.to_bytes(), hex32(MASTER_SECRET));
    assert_eq!(recovered.hpke.public_key(), body.recovery);
    assert!(
        open_recovery(&record, &backup, &key("0123-4567-89AC")).is_err(),
        "the backup vector opens with another key"
    );
}
