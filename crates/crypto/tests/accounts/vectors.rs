//! The vectors of `docs/protocol/test-vectors.md`, "Account record" and "Master backup", which
//! decode and encode back to the published bytes. The record, at revision 1, is that of the
//! account of the other vectors, its master key RFC 8032's "TEST 1" key and its recovery key
//! `0123-4567-89AB`. The backup seals that master private key with that recovery key under the
//! nonce of the 12 bytes 00 to 0b, so its bytes are fixed: the code under test gives them byte for
//! byte with that nonce, and they open as "Account" says.

use skald_crypto::{
    RecoveryKey, SigningKey, derive_at_spec_cost, new_recovery_with_nonce, open_recovery,
    sign_account,
};
use skald_proto::{AccountBody, AccountId, AccountRecord, Cbor, MasterBackup};

use crate::common::{hex, hex32};
use crate::fixtures::{derived, open_as_specified};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The account of the vectors.
const ACCOUNT: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9";
/// RFC 8032, §7.1, "TEST 1": the secret key and its public key.
const MASTER_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const MASTER_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
/// The recovery key, in its shown form.
const RECOVERY: &str = "0123-4567-89AB";
/// The backup's nonce: the 12 bytes 00 to 0b.
const NONCE: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

fn account() -> AccountId {
    ACCOUNT.parse().expect("an account ID")
}

/// The bytes of the first `hex` block in the section `## <name>` of the document.
fn published(name: &str) -> Vec<u8> {
    let doc = DOC.replace("\r\n", "\n");
    let section = doc
        .split("\n## ")
        .find(|section| section.lines().next() == Some(name))
        .unwrap_or_else(|| panic!("test-vectors.md has no section {name:?}"));
    let block = section
        .split_once("```hex\n")
        .and_then(|(_, rest)| rest.split_once("```"))
        .map(|(block, _)| block)
        .unwrap_or_else(|| panic!("{name:?} has no hex block"));
    hex(block)
}

/// The vectors' record and backup: those the published bytes encode.
fn vectors() -> (AccountRecord, MasterBackup) {
    let record = published("Account record");
    let backup = published("Master backup");
    let decoded = (
        AccountRecord::from_cbor(&record).expect("the record vector decodes"),
        MasterBackup::from_cbor(&backup).expect("the backup vector decodes"),
    );
    assert_eq!(decoded.0.to_cbor(), record, "the record, byte for byte");
    assert_eq!(decoded.1.to_cbor(), backup, "the backup, byte for byte");
    decoded
}

/// The record names the account, "TEST 1" and the recovery HPKE key the recovery key derives on
/// it; the backup opens, as "Account" says, to the private key "TEST 1" and with the code under
/// test to it and to that recovery HPKE key.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_are_derived_and_sealed_as_the_spec_says() {
    derive_at_spec_cost();
    let (record, backup) = vectors();
    let (aead, recovery_public) = derived(RECOVERY, account());

    let body = record.body();
    assert_eq!(body.account, account());
    assert_eq!(body.master, hex32(MASTER_PUBLIC));
    assert_eq!(body.recovery, recovery_public);
    assert_eq!(body.revision, 1);
    let alone = AccountBody::from_cbor(record.body_bytes()).expect("the body decodes");
    assert_eq!(
        alone.to_cbor(),
        record.body_bytes(),
        "the body, byte for byte"
    );
    assert_eq!(backup.nonce, NONCE);
    let opened = open_as_specified(&backup, &aead, account());
    assert_eq!(opened, Some(hex(MASTER_SECRET)));
    let recovery = RecoveryKey::parse(RECOVERY).expect("a recovery key");
    let recovered =
        open_recovery(&record, &backup, &recovery).expect("the vector opens with its recovery key");
    assert_eq!(*recovered.master.to_bytes(), hex32(MASTER_SECRET));
    assert_eq!(recovered.hpke.public_key(), recovery_public);
}

/// The code under test makes the vectors byte for byte with their nonce.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_recovery_made_with_the_vectors_nonce_matches_them_byte_for_byte() {
    derive_at_spec_cost();
    let master = SigningKey::from_bytes(&hex32(MASTER_SECRET));
    assert_eq!(master.public_key(), hex32(MASTER_PUBLIC));
    let recovery = RecoveryKey::parse(RECOVERY).expect("a recovery key");

    let (body, backup) = new_recovery_with_nonce(&master, &recovery, account(), NONCE);

    let record = sign_account(&body, &master).expect("a valid record");
    assert_eq!(record.to_cbor(), published("Account record"));
    assert_eq!(backup.to_cbor(), published("Master backup"));
}
