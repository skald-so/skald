//! Master backups: `master-backup = [nonce, ct]`, the 32-byte master private key sealed with
//! AES-256-GCM under the AEAD key Argon2id derives from the recovery key on the account, `aad` the
//! account ID; the same derivation gives the recovery HPKE key pair the account record names. The
//! recovery key unlocks it; the opened key must match `master`.

use skald_crypto::{
    AccountError, SigningKey, derive_at_spec_cost, new_recovery, open_recovery, sign_account,
};
use skald_proto::{AccountBody, AccountRecord, Cbor, MasterBackup};

use crate::cbor::{Value, bytes, text};
use crate::fields::truncated_and_longer;
use crate::fixtures::{
    ACCOUNT, MASTER, OTHER_ACCOUNT, OTHER_MASTER, OTHER_RECOVERY, RECOVERY, derived, master,
    open_as_specified, public, recovery_key, seal_as_specified,
};

/// The account record of `body`, signed by the account's master key.
fn record_of(body: &AccountBody) -> AccountRecord {
    sign_account(body, &master()).expect("a valid record")
}

/// The account's record and backup of [`RECOVERY`], made by the code under test.
fn made() -> (AccountRecord, MasterBackup) {
    let (body, backup) = new_recovery(&master(), &recovery_key(RECOVERY), ACCOUNT);
    (record_of(&body), backup)
}

/// Why `backup` with `record` does not open with the recovery key `shown`, if it does not.
fn refused(record: &AccountRecord, backup: &MasterBackup, shown: &str) -> Option<AccountError> {
    open_recovery(record, backup, &recovery_key(shown)).err()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_and_record_are_derived_and_sealed_as_the_spec_says() {
    derive_at_spec_cost();
    let (record, backup) = made();
    let (aead, recovery_public) = derived(RECOVERY, ACCOUNT);

    let body = record.body();
    assert_eq!(
        (body.account, body.master, body.recovery, body.revision),
        (ACCOUNT, public(&MASTER), recovery_public, 1)
    );
    assert_eq!(
        open_as_specified(&backup, &aead, ACCOUNT).as_deref(),
        Some(MASTER.as_slice())
    );
    let encoded = backup.to_cbor();
    assert_eq!(
        encoded,
        Value::Array(vec![bytes(&backup.nonce), bytes(&backup.ct)]).encode()
    );
    assert_eq!(MasterBackup::from_cbor(&encoded), Ok(backup));
    // And one sealed so by the tests opens with the code under test.
    let nonce = [0x5a; 12];
    let theirs = MasterBackup {
        nonce,
        ct: seal_as_specified(&MASTER, &aead, ACCOUNT, nonce)
            .try_into()
            .expect("the key and the tag"),
    };
    let recovered = open_recovery(&record, &theirs, &recovery_key(RECOVERY)).expect("it opens");
    assert_eq!(*recovered.master.to_bytes(), MASTER);
    assert_eq!(recovered.hpke.public_key(), recovery_public);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_backup_takes_a_new_nonce() {
    derive_at_spec_cost();
    let recovery = recovery_key(RECOVERY);
    let (_, one) = new_recovery(&master(), &recovery, ACCOUNT);
    let (_, two) = new_recovery(&master(), &recovery, ACCOUNT);

    assert_ne!(one.nonce, two.nonce);
    assert_ne!(one.ct, two.ct);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_opens_only_with_its_recovery_key_and_account() {
    derive_at_spec_cost();
    let (record, backup) = made();

    assert_eq!(
        refused(&record, &backup, OTHER_RECOVERY),
        Some(AccountError::Open),
        "another recovery key"
    );
    let mut elsewhere = record.body().clone();
    elsewhere.account = OTHER_ACCOUNT;
    assert_eq!(
        refused(&record_of(&elsewhere), &backup, RECOVERY),
        Some(AccountError::Open),
        "another account"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_that_opens_to_a_key_other_than_master_is_refused() {
    derive_at_spec_cost();
    let (record, _) = made();
    let (other_body, other) = new_recovery(
        &SigningKey::from_bytes(&OTHER_MASTER),
        &recovery_key(RECOVERY),
        ACCOUNT,
    );
    assert_eq!(other_body.recovery, record.body().recovery);

    assert_eq!(
        refused(&record, &other, RECOVERY),
        Some(AccountError::Key),
        "another master key"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_record_that_names_another_recovery_key_is_refused() {
    derive_at_spec_cost();
    let (record, backup) = made();
    let mut body = record.body().clone();
    body.recovery[0] ^= 0x01;

    assert_eq!(
        refused(&record_of(&body), &backup, RECOVERY),
        Some(AccountError::Key)
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_backup_does_not_open() {
    derive_at_spec_cost();
    let (record, backup) = made();
    let mut changed = Vec::new();
    for at in [0, 16, backup.ct.len() - 1] {
        let mut one = backup.clone();
        one.ct[at] ^= 0x01;
        changed.push((format!("ct byte {at}"), one));
    }
    let mut one = backup.clone();
    one.nonce[5] ^= 0x01;
    changed.push(("the nonce".to_owned(), one));
    for (what, backup) in &changed {
        assert_eq!(
            refused(&record, backup, RECOVERY),
            Some(AccountError::Open),
            "{what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_backups_are_refused() {
    derive_at_spec_cost();
    let nonce = [0xe0; 12];
    let ct = [0xc0; 48];
    let valid = Value::Array(vec![bytes(&nonce), bytes(&ct)]).encode();
    assert!(MasterBackup::from_cbor(&valid).is_ok());
    let mut inputs = vec![
        Value::Array(vec![bytes(&nonce[..11]), bytes(&ct)]).encode(),
        Value::Array(vec![bytes(&[0xe0; 13]), bytes(&ct)]).encode(),
        Value::Array(vec![bytes(&nonce), bytes(&ct[..47])]).encode(),
        Value::Array(vec![bytes(&nonce), bytes(&[0xc0; 49])]).encode(),
        Value::Array(vec![bytes(&nonce), text("ct")]).encode(),
        Value::Array(vec![bytes(&nonce)]).encode(),
        Value::Array(vec![bytes(&nonce), bytes(&ct), bytes(&ct)]).encode(),
        Value::Map(vec![(0, bytes(&nonce)), (1, bytes(&ct))]).encode(),
        bytes(&ct).encode(),
        // The form before: a 32-byte encapsulated key.
        Value::Array(vec![bytes(&[0xe0; 32]), bytes(&ct)]).encode(),
    ];
    inputs.extend(truncated_and_longer(&valid));
    for input in &inputs {
        assert!(MasterBackup::from_cbor(input).is_err(), "{input:?}");
    }
}
