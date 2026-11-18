//! Master backups: `master-backup = [enc, ct]`, the 32-byte master private key sealed with HPKE in
//! PSK mode, single-shot, to the recovery HPKE key, `psk` derived from the recovery key, `psk_id`
//! and `info` `skald master backup`, `aad` the account ID. The recovery key unlocks it; the opened
//! key must match `master`.

use skald_crypto::{HpkeKey, RecoveryKey, SigningKey, open_master_backup, seal_master_backup};
use skald_proto::{Cbor, MasterBackup};

use crate::cbor::{Value, bytes, text};
use crate::fields::truncated_and_longer;
use crate::fixtures::{
    ACCOUNT, BACKUP_INFO, MASTER, OTHER_ACCOUNT, OTHER_MASTER, OTHER_RECOVERY, RECOVERY, master,
    open_as_specified, public, recovery_public, seal_as_specified,
};

fn recovery() -> RecoveryKey {
    RecoveryKey::from_bytes(&RECOVERY)
}

/// Whether `backup` opens with `recovery` on `account` to the account's master key.
fn opens(backup: &MasterBackup, recovery: &[u8; 32], account: skald_proto::AccountId) -> bool {
    open_master_backup(
        backup,
        &RecoveryKey::from_bytes(recovery),
        account,
        &public(&MASTER),
    )
    .is_ok()
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_opens_with_its_recovery_key_to_the_master_key() {
    let backup = seal_master_backup(&master(), &recovery(), ACCOUNT);

    let opened =
        open_master_backup(&backup, &recovery(), ACCOUNT, &public(&MASTER)).expect("it opens");

    assert_eq!(opened.public_key(), public(&MASTER));
    assert_eq!(*opened.to_bytes(), MASTER);
    assert_eq!(backup.ct.len(), 32 + 16, "the key and the AES-GCM tag");
    let encoded = backup.to_cbor();
    assert_eq!(
        encoded,
        Value::Array(vec![bytes(&backup.enc), bytes(&backup.ct)]).encode()
    );
    assert_eq!(MasterBackup::from_cbor(&encoded), Ok(backup));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_is_sealed_in_psk_mode_as_the_spec_says() {
    let backup = seal_master_backup(&master(), &recovery(), ACCOUNT);

    let opened = open_as_specified(&backup.enc, &backup.ct, &RECOVERY, ACCOUNT);

    assert_eq!(opened.as_deref(), Some(MASTER.as_slice()));
    let base = HpkeKey::from_recovery_key(&RECOVERY).open(
        &backup.enc,
        BACKUP_INFO,
        ACCOUNT.as_bytes(),
        &backup.ct,
    );
    assert!(base.is_err(), "it opens in base mode");
    // And one sealed so by the tests opens with the code under test.
    let theirs = seal_as_specified(&MASTER, &RECOVERY, ACCOUNT);
    assert!(opens(&theirs, &RECOVERY, ACCOUNT));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn each_seal_takes_a_new_ephemeral_key() {
    let one = seal_master_backup(&master(), &recovery(), ACCOUNT);
    let two = seal_master_backup(&master(), &recovery(), ACCOUNT);

    assert_ne!(one.enc, two.enc);
    assert_ne!(one.ct, two.ct);
    assert!(opens(&one, &RECOVERY, ACCOUNT) && opens(&two, &RECOVERY, ACCOUNT));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_opens_only_with_its_recovery_key_and_account() {
    let backup = seal_master_backup(&master(), &recovery(), ACCOUNT);

    assert!(opens(&backup, &RECOVERY, ACCOUNT));
    assert!(
        !opens(&backup, &OTHER_RECOVERY, ACCOUNT),
        "another recovery key"
    );
    assert!(!opens(&backup, &RECOVERY, OTHER_ACCOUNT), "another account");
    // The recovery HPKE private key itself is not the recovery key.
    let derived = *HpkeKey::from_recovery_key(&RECOVERY).to_bytes();
    assert!(
        !opens(&backup, &derived, ACCOUNT),
        "the derived private key"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_sealed_in_base_mode_to_the_recovery_hpke_key_does_not_open() {
    let (enc, ct) = HpkeKey::seal(
        &recovery_public(&RECOVERY),
        BACKUP_INFO,
        ACCOUNT.as_bytes(),
        &MASTER,
    )
    .expect("a seal to the recovery HPKE key");

    assert!(!opens(&MasterBackup { enc, ct }, &RECOVERY, ACCOUNT));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_that_opens_to_a_key_other_than_master_is_refused() {
    let other = seal_master_backup(&SigningKey::from_bytes(&OTHER_MASTER), &recovery(), ACCOUNT);
    assert!(
        open_master_backup(&other, &recovery(), ACCOUNT, &public(&OTHER_MASTER)).is_ok(),
        "it opens to its own master key"
    );

    assert!(!opens(&other, &RECOVERY, ACCOUNT), "another master key");
    let plaintexts = [
        MASTER[..31].to_vec(),
        [&MASTER[..], &[0]].concat(),
        Vec::new(),
    ];
    for plaintext in &plaintexts {
        let odd = seal_as_specified(plaintext, &RECOVERY, ACCOUNT);
        assert!(
            !opens(&odd, &RECOVERY, ACCOUNT),
            "{} bytes",
            plaintext.len()
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_changed_backup_does_not_open() {
    let backup = seal_master_backup(&master(), &recovery(), ACCOUNT);
    for at in [0, 16, backup.ct.len() - 1] {
        let mut changed = backup.clone();
        changed.ct[at] ^= 0x01;
        assert!(!opens(&changed, &RECOVERY, ACCOUNT), "ct byte {at}");
    }
    let mut changed = backup.clone();
    changed.enc[5] ^= 0x01;
    assert!(!opens(&changed, &RECOVERY, ACCOUNT), "enc");
    let mut cut = backup;
    cut.ct.pop();
    assert!(!opens(&cut, &RECOVERY, ACCOUNT), "a cut ct");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_backups_are_refused() {
    let enc = [0xe0; 32];
    let ct = [0xc0; 48];
    let valid = Value::Array(vec![bytes(&enc), bytes(&ct)]).encode();
    assert!(MasterBackup::from_cbor(&valid).is_ok());
    let mut inputs = vec![
        Value::Array(vec![bytes(&enc[..31]), bytes(&ct)]).encode(),
        Value::Array(vec![bytes(&[0xe0; 33]), bytes(&ct)]).encode(),
        Value::Array(vec![bytes(&enc), text("ct")]).encode(),
        Value::Array(vec![bytes(&enc)]).encode(),
        Value::Array(vec![bytes(&enc), bytes(&ct), bytes(&ct)]).encode(),
        Value::Map(vec![(0, bytes(&enc)), (1, bytes(&ct))]).encode(),
        bytes(&ct).encode(),
    ];
    inputs.extend(truncated_and_longer(&valid));
    for input in &inputs {
        assert!(MasterBackup::from_cbor(input).is_err(), "{input:?}");
    }
}
