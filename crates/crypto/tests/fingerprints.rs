//! Account fingerprints, natively and in WASM: `docs/protocol/membership.md`, "Account
//! fingerprint". A fingerprint is at least 128 bits of SHA-256 over a label, the account ID and its
//! master key, shown as digits in groups of five; two people compare the ones their devices show.
//! The vector of `docs/protocol/test-vectors.md`, "Account fingerprint", is checked against the
//! code.
//!
//! The API, as these tests use it:
//! - `skald_crypto::account_fingerprint(AccountId, master: &[u8; 32]) -> String`: the shown form,
//!   groups of five ASCII digits separated by one space each.
//!
//! The vector's account is that of the other vectors, its master key RFC 8032's "TEST 1" public
//! key, as in "Account record"; its section shows the fingerprint as digits in groups of five.

#[allow(dead_code)]
mod common;

use skald_crypto::account_fingerprint;
use skald_proto::AccountId;

use common::hex32;

const DOC: &str = include_str!("../../../docs/protocol/test-vectors.md");

/// The account of the vectors.
const ACCOUNT: &str = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e9";
/// RFC 8032, §7.1, "TEST 1": the public key.
const MASTER: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

/// The fewest digits that hold 128 bits: 10^39 > 2^128 > 10^38.
const MIN_DIGITS: usize = 39;

fn account() -> AccountId {
    ACCOUNT.parse().expect("an account ID")
}

/// The text of the section `## <name>` of the document.
fn section(name: &str) -> String {
    let doc = DOC.replace("\r\n", "\n");
    doc.split("\n## ")
        .find(|section| section.lines().next() == Some(name))
        .unwrap_or_else(|| panic!("test-vectors.md has no section {name:?}"))
        .to_owned()
}

/// The longest run in `text` of words that are five ASCII digits each, joined by one space.
fn digit_groups(text: &str) -> String {
    let mut best: Vec<&str> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == '`') {
        if word.is_empty() {
            continue;
        }
        if word.len() == 5 && word.bytes().all(|byte| byte.is_ascii_digit()) {
            run.push(word);
            if run.len() > best.len() {
                best = run.clone();
            }
        } else {
            run.clear();
        }
    }
    best.join(" ")
}

/// Fails unless `shown` is groups of five ASCII digits, one space between each two, holding at
/// least 128 bits.
#[track_caller]
fn assert_shown_form(shown: &str) {
    let groups: Vec<&str> = shown.split(' ').collect();
    assert!(
        groups
            .iter()
            .all(|group| group.len() == 5 && group.bytes().all(|byte| byte.is_ascii_digit())),
        "not digits in groups of five: {shown:?}"
    );
    assert!(
        groups.len() * 5 >= MIN_DIGITS,
        "fewer than {MIN_DIGITS} digits, 128 bits: {shown:?}"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn the_fingerprint_matches_its_test_vector() {
    let published = digit_groups(&section("Account fingerprint"));
    assert_shown_form(&published);

    let computed = account_fingerprint(account(), &hex32(MASTER));

    assert_eq!(computed, published);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_fingerprint_is_the_same_digits_in_groups_of_five_holding_at_least_128_bits() {
    let shown = account_fingerprint(account(), &hex32(MASTER));

    assert_shown_form(&shown);
    assert_eq!(
        shown,
        account_fingerprint(account(), &hex32(MASTER)),
        "the same account and master key give another"
    );
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn another_account_id_gives_another_fingerprint() {
    let master = hex32(MASTER);
    let other: AccountId = "019a6f1e-2b3c-7d4e-8f50-61a2b3c4d5e8"
        .parse()
        .expect("an account ID");

    let theirs = account_fingerprint(other, &master);

    assert_shown_form(&theirs);
    assert_ne!(theirs, account_fingerprint(account(), &master));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn another_master_key_gives_another_fingerprint() {
    let mut other = hex32(MASTER);
    other[31] ^= 1;

    let theirs = account_fingerprint(account(), &other);

    assert_shown_form(&theirs);
    assert_ne!(theirs, account_fingerprint(account(), &hex32(MASTER)));
}

/// Fingerprints of different accounts and master keys have one length and differ from each other.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn fingerprints_of_different_accounts_have_one_length_and_differ() {
    let shown: Vec<String> = (0..16u8)
        .map(|seed| {
            let account = AccountId::generate();
            account_fingerprint(account, &[seed; 32])
        })
        .collect();

    for (at, one) in shown.iter().enumerate() {
        assert_shown_form(one);
        assert_eq!(one.len(), shown[0].len(), "{shown:?}");
        assert!(
            !shown[..at].contains(one),
            "a repeated fingerprint: {shown:?}"
        );
    }
}
