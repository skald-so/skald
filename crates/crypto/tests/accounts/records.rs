//! Account records: `account = signed<account-body>`, signed by the master key over
//! `["skald account", body]`. A record keeps its body's bytes as it got them, unknown fields
//! included; a client rejects one whose signature does not verify with `master` or whose `master`
//! is not the one it pinned. Malformed and truncated input is refused, never a panic.

use skald_crypto::{SigningKey, check_account, sign_account};
use skald_proto::{ACCOUNT_LABEL, AccountBody, AccountRecord, Cbor, Unknown};

use crate::cbor::{Value, bytes, text};
use crate::fields::{map, signed_cbor, truncated_and_longer, unknown, with, without};
use crate::fixtures::{
    ACCOUNT, LABEL, MASTER, OTHER_ACCOUNT, OTHER_MASTER, RECOVERY_PUBLIC, account_fields,
    body_fields, master, public,
};

/// The tests' account body: the account's master key and recovery key, revision 1.
fn body() -> AccountBody {
    AccountBody {
        account: ACCOUNT,
        master: public(&MASTER),
        recovery: RECOVERY_PUBLIC,
        revision: 1,
        unknown: Unknown::default(),
    }
}

/// The record of the encoded `body` that `key` signs over `label`.
fn signed(body: &[u8], key: &SigningKey, label: &str) -> AccountRecord {
    let signature = key.sign(label, &[body]);
    AccountRecord::from_parts(body.to_vec(), signature).expect("an account record")
}

/// Fails unless `record` encodes to `encoded` byte for byte, decodes back from it, and passes the
/// check with the account's master key.
#[track_caller]
fn assert_encodes(record: &AccountRecord, encoded: &[u8]) {
    assert_eq!(record.to_cbor(), encoded, "the record, byte for byte");
    assert_eq!(AccountRecord::from_cbor(encoded).as_ref(), Ok(record));
    assert!(
        check_account(record, &public(&MASTER)).is_ok(),
        "the master key's check"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_label_of_an_account_record_is_skald_account() {
    assert_eq!(ACCOUNT_LABEL, LABEL);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_record_the_master_key_signs_encodes_its_fields_and_passes_the_check() {
    let record = sign_account(&body(), &master()).expect("a record");

    assert_eq!(record.body(), &body());
    assert_eq!(record.body_bytes(), map(&account_fields()).as_slice());
    let signature = master().sign(LABEL, &[record.body_bytes()]);
    assert_eq!(
        record.signature(),
        &signature,
        "the master key over its label"
    );
    assert_encodes(&record, &signed_cbor(record.body_bytes(), &signature));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_record_with_fields_of_later_versions_round_trips_byte_for_byte_and_passes_the_check() {
    let mut fields = body_fields(ACCOUNT, &public(&MASTER), &RECOVERY_PUBLIC, 7);
    fields.extend(unknown());
    let body_bytes = map(&fields);
    let signature = master().sign(LABEL, &[&body_bytes]);
    let encoded = signed_cbor(&body_bytes, &signature);

    let record = AccountRecord::from_cbor(&encoded).expect("a record");

    assert_encodes(&record, &encoded);
    assert_eq!(record.body_bytes(), body_bytes.as_slice());
    let decoded = record.body();
    assert_eq!(decoded.account, ACCOUNT);
    assert_eq!(decoded.master, public(&MASTER));
    assert_eq!(decoded.recovery, RECOVERY_PUBLIC);
    assert_eq!(decoded.revision, 7);
    assert_ne!(
        decoded.unknown,
        Unknown::default(),
        "the later fields are kept"
    );
    let body = AccountBody::from_cbor(&body_bytes).expect("an account body");
    assert_eq!(body.to_cbor(), body_bytes, "the body alone, byte for byte");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_record_that_the_pinned_master_key_does_not_sign_or_that_names_another_is_refused() {
    let body = map(&account_fields());
    let other = SigningKey::from_bytes(&OTHER_MASTER);
    let genuine = signed(&body, &master(), LABEL);
    let mut broken = *genuine.signature();
    broken[10] ^= 0x01;
    let naming_other = with(&account_fields(), 2, bytes(&public(&OTHER_MASTER)));
    let refused = [
        (
            "a broken signature",
            AccountRecord::from_parts(body.clone(), broken).expect("a record"),
        ),
        ("signed by another key", signed(&body, &other, LABEL)),
        (
            "signed over another label",
            signed(&body, &master(), "skald cert"),
        ),
        (
            "naming another master key, signed by the pinned one",
            signed(&naming_other, &master(), LABEL),
        ),
        (
            "naming another master key, signed by it",
            signed(&naming_other, &other, LABEL),
        ),
    ];

    assert!(check_account(&genuine, &public(&MASTER)).is_ok());
    assert!(
        check_account(&genuine, &public(&OTHER_MASTER)).is_err(),
        "checked against another pinned key"
    );
    for (what, record) in &refused {
        assert!(check_account(record, &public(&MASTER)).is_err(), "{what}");
    }
    // A record of another account still is one: the account it names is the caller's to check.
    let elsewhere = with(&account_fields(), 1, bytes(OTHER_ACCOUNT.as_bytes()));
    let record = signed(&elsewhere, &master(), LABEL);
    assert_eq!(record.body().account, OTHER_ACCOUNT);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_records_are_refused() {
    let fields = account_fields();
    let mut bodies = vec![
        ("no account", without(&fields, 1)),
        ("no master", without(&fields, 2)),
        ("no recovery", without(&fields, 3)),
        ("no revision", without(&fields, 4)),
        (
            "an account of 15 bytes",
            with(&fields, 1, bytes(&[0xac; 15])),
        ),
        ("a master of 31 bytes", with(&fields, 2, bytes(&[0x4d; 31]))),
        ("a master of 33 bytes", with(&fields, 2, bytes(&[0x4d; 33]))),
        (
            "a recovery of 31 bytes",
            with(&fields, 3, bytes(&[0x7e; 31])),
        ),
        ("a recovery as text", with(&fields, 3, text("recovery"))),
        ("revision 0", with(&fields, 4, Value::Uint(0))),
        ("a revision as text", with(&fields, 4, text("1"))),
        (
            "an array",
            Value::Array(fields.iter().map(|(_, value)| value.clone()).collect()).encode(),
        ),
    ];
    for (at, cut) in truncated_and_longer(&map(&fields)).into_iter().enumerate() {
        bodies.push((if at == 0 { "empty" } else { "cut or longer" }, cut));
    }
    for (what, body) in &bodies {
        assert!(AccountBody::from_cbor(body).is_err(), "{what}: {body:?}");
        let record = signed_cbor(body, &[0x5a; 64]);
        assert!(
            AccountRecord::from_cbor(&record).is_err(),
            "a record with {what}"
        );
    }

    let record = sign_account(&body(), &master())
        .expect("a record")
        .to_cbor();
    for cut in truncated_and_longer(&record) {
        assert!(AccountRecord::from_cbor(&cut).is_err(), "{cut:?}");
    }
    let short_signature = signed_cbor(&map(&fields), &[0x5a; 63]);
    assert!(AccountRecord::from_cbor(&short_signature).is_err());
}
