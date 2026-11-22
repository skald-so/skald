//! A space's policy naming its organization's domain, natively and in WASM (`docs/protocol/
//! membership.md`, "Policy"): the optional `domain`, a text string at key 4 of `policy`, in
//! `skald-proto`; a new space's policy names none; a revision that sets or clears it is checked as
//! any other.
//!
//! The fixtures are those of the tests of rosters (`rosters/fixtures.rs`): revisions encoded from
//! their spec fields and signed apart from the crates under test.

#[path = "../rosters/cbor.rs"]
mod cbor;
#[allow(dead_code)]
#[path = "../rosters/fixtures.rs"]
mod fixtures;

use skald_proto::{Cbor, Roster, RosterBody};

use cbor::{Value, bytes, text};
use fixtures::*;

/// The key of `domain` in `policy`.
const DOMAIN_FIELD: i64 = 4;

/// The domain the fixtures name.
const DOMAIN: &str = "corp-q7k2.example";

/// Map fields of a later version in the policy, which decoding keeps and encoding gives back.
fn later_fields() -> Vec<(i64, Value)> {
    vec![(9, text("later")), (-2, bytes(b"later"))]
}

/// [`current`] with its policy's `domain` set to `domain` and the policy's fields of later versions
/// `later`.
fn with_domain(domain: Option<Value>, later: Vec<(i64, Value)>) -> Rev {
    let mut rev = current();
    rev.policy_extra = later;
    rev.policy_extra
        .extend(domain.map(|domain| (DOMAIN_FIELD, domain)));
    rev
}

/// Checks that `rev`'s encoded body decodes into a body whose policy names `expected`, and that it
/// and its roster encode back byte for byte.
#[track_caller]
fn assert_round_trip(rev: &Rev, expected: Option<&str>) {
    let body = RosterBody::from_cbor(&rev.body()).expect("the body decodes");
    assert_eq!(body.policy.domain.as_deref(), expected);
    assert_eq!(body.to_cbor(), rev.body(), "the body, byte for byte");
    let roster = Roster::from_cbor(&rev.bytes()).expect("the roster decodes");
    assert_eq!(roster.body(), &body);
    assert_holds(&roster, rev);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_policy_with_a_domain_round_trips_byte_for_byte() {
    for later in [Vec::new(), later_fields()] {
        let rev = with_domain(Some(text(DOMAIN)), later.clone());
        assert_round_trip(&rev, Some(DOMAIN));
        let body = RosterBody::from_cbor(&rev.body()).unwrap();
        let mut keys: Vec<i128> = body.policy.unknown.keys().collect();
        keys.sort_unstable();
        let mut expected: Vec<i128> = later.iter().map(|(key, _)| i128::from(*key)).collect();
        expected.sort_unstable();
        assert_eq!(keys, expected, "the policy's fields of later versions");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_policy_without_a_domain_round_trips_byte_for_byte() {
    for later in [Vec::new(), later_fields()] {
        assert_round_trip(&with_domain(None, later), None);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_domain_set_or_cleared_in_the_type_encodes_at_key_4() {
    let without = RosterBody::from_cbor(&with_domain(None, later_fields()).body()).unwrap();
    let mut set = without.clone();
    set.policy.domain = Some(DOMAIN.to_owned());
    assert_eq!(
        set.to_cbor(),
        with_domain(Some(text(DOMAIN)), later_fields()).body(),
        "the domain is not a text string at key 4 of the policy"
    );
    let mut cleared = set;
    cleared.policy.domain = None;
    assert_eq!(cleared.to_cbor(), without.to_cbor());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_domain_that_is_no_text_string_is_refused() {
    for (what, domain) in [
        ("bytes", bytes(DOMAIN.as_bytes())),
        ("a number", Value::Uint(4)),
        ("an array", Value::Array(vec![text(DOMAIN)])),
    ] {
        let rev = with_domain(Some(domain), Vec::new());
        assert!(RosterBody::from_cbor(&rev.body()).is_err(), "{what}");
        assert!(Roster::from_cbor(&rev.bytes()).is_err(), "{what}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_new_spaces_policy_names_no_domain() {
    assert_eq!(first_body().policy.domain, None, "built by the crate");
    let decoded = RosterBody::from_cbor(&revision_one().body()).unwrap();
    assert_eq!(decoded.policy.domain, None, "decoded");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_that_sets_or_clears_the_domain_is_accepted() {
    let mut set = successor(&current());
    set.policy_extra = vec![(DOMAIN_FIELD, text(DOMAIN))];
    assert_eq!(accepts(&set.roster()), Ok(()), "setting the domain");

    let set_roster = set.roster();
    assert_eq!(set_roster.body().policy.domain.as_deref(), Some(DOMAIN));
    let mut cleared = successor(&set);
    cleared.policy_extra.clear();
    assert_eq!(
        accepts_after(&set, &cleared.roster()),
        Ok(()),
        "clearing the domain"
    );
    assert_eq!(cleared.roster().body().policy.domain, None);
}
