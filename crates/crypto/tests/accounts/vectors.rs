//! The vectors of `docs/protocol/test-vectors.md`, "Account record" and "Master backup", which
//! decode and encode back to the published bytes. The record, at revision 1, is that of the
//! account of the other vectors, its master key RFC 8032's "TEST 1" key and its recovery key the 32
//! bytes 00 to 1f. The backup seals that master private key to that recovery key with the
//! ephemeral key pair `DeriveKeyPair(ikmE)`, `ikmE` that of RFC 9180, Appendix A.1.2, so its bytes
//! are fixed: sealing with that ephemeral key gives them byte for byte, and they open as "Account"
//! says.

use skald_crypto::{HpkeKey, RecoveryKey, SigningKey, open_master_backup};
use skald_proto::{AccountBody, AccountId, AccountRecord, Cbor, MasterBackup};

use crate::common::{hex, hex32};
use crate::fixtures::{open_as_specified, recovery_public};

const DOC: &str = include_str!("../../../../docs/protocol/test-vectors.md");

/// The account of the vectors.
const ACCOUNT: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9";
/// RFC 8032, §7.1, "TEST 1": the secret key and its public key.
const MASTER_SECRET: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const MASTER_PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
/// The recovery key: 32 bytes 00 to 1f.
const RECOVERY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
/// RFC 9180, Appendix A.1.2: `ikmE`.
const EPHEMERAL: &str = "78628c354e46f3e169bd231be7b2ff1c77aa302460a26dbfa15515684c00130b";

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

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_an_account_record() {
    let encoded = published("Account record");

    let record = AccountRecord::from_cbor(&encoded).expect("the vector decodes");

    assert_eq!(record.to_cbor(), encoded, "byte for byte");
    let body = record.body();
    assert_eq!(body.account, account());
    assert_eq!(body.master, hex32(MASTER_PUBLIC));
    assert_eq!(body.recovery, recovery_public(&hex32(RECOVERY)));
    assert_eq!(body.revision, 1);
    let alone = AccountBody::from_cbor(record.body_bytes()).expect("the body decodes");
    assert_eq!(
        alone.to_cbor(),
        record.body_bytes(),
        "the body, byte for byte"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_test_vectors_have_a_master_backup_that_opens_as_the_spec_says() {
    let encoded = published("Master backup");

    let backup = MasterBackup::from_cbor(&encoded).expect("the vector decodes");

    assert_eq!(backup.to_cbor(), encoded, "byte for byte");
    assert_eq!(
        backup.enc,
        HpkeKey::from_recovery_key(&hex32(EPHEMERAL)).public_key(),
        "enc is the public key of DeriveKeyPair(ikmE)"
    );
    let opened = open_as_specified(&backup.enc, &backup.ct, &hex32(RECOVERY), account());
    assert_eq!(opened, Some(hex(MASTER_SECRET)));
    let recovery = RecoveryKey::from_bytes(&hex32(RECOVERY));
    let key = open_master_backup(&backup, &recovery, account(), &hex32(MASTER_PUBLIC))
        .expect("the vector opens with its recovery key");
    assert_eq!(key.public_key(), hex32(MASTER_PUBLIC));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_backup_sealed_with_the_vectors_ephemeral_key_matches_it_byte_for_byte() {
    let master = SigningKey::from_bytes(&hex32(MASTER_SECRET));
    assert_eq!(master.public_key(), hex32(MASTER_PUBLIC));
    let recovery = RecoveryKey::from_bytes(&hex32(RECOVERY));

    let sealed = skald_crypto::seal_master_backup_with_ephemeral(
        &master,
        &recovery,
        account(),
        &hex32(EPHEMERAL),
    );

    assert_eq!(sealed.to_cbor(), published("Master backup"));
}
