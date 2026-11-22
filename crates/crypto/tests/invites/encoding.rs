//! The invite and acceptance types of `skald-proto`: `membership.md`, "Invite", encoded as "Signed
//! objects" and `README.md`, "Encoding", say. Each keeps its body's bytes as it got them, unknown
//! fields included. Malformed and truncated input is refused, never a panic.

use std::fmt::Debug;

use skald_proto::{
    Acceptance, AcceptanceBody, Cbor, Invite, InviteBody, Role, Signed, SignedBody, Unknown,
};

use crate::cbor::{Value, bytes, text};
use crate::fields;
use crate::objects::{Acc, GENERAL, Inv, Object, assert_is};

/// The tests' invites without fields of later versions.
fn invites() -> [Inv; 2] {
    [Inv::of_member(), Inv::of_admin_to_no_channel()]
}

/// The tests' acceptances without fields of later versions.
fn acceptances() -> [Acc; 2] {
    [Acc::of_newcomer(), Acc::with_other_handle()]
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_decodes_into_its_spec_fields() {
    for inv in invites() {
        let invite = Invite::from_cbor(&inv.bytes()).unwrap();
        let body = invite.body();
        assert_eq!(body.invite, inv.invite);
        assert_eq!(body.space, inv.space);
        assert_eq!(body.key, inv.key);
        assert_eq!(body.role, inv.role);
        assert_eq!(body.channels, inv.channels, "the channels, in order");
        assert_eq!(body.signer, inv.signer_id());
        assert_eq!(body.expires, inv.expires);
        assert!(body.unknown.is_empty(), "{:?}", body.unknown);
        assert_is(&invite, &inv);
    }
    let roles = invites().map(|inv| Invite::from_cbor(&inv.bytes()).unwrap().body().role);
    assert_eq!(roles, [Role::Member, Role::Admin]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_body_encodes_to_its_spec_bytes() {
    for inv in invites() {
        let body = inv.proto_body();
        assert_eq!(body.to_cbor(), inv.body(), "{inv:?}");
        assert_eq!(InviteBody::from_cbor(&inv.body()), Ok(body), "{inv:?}");
    }
}

/// Checks that `unknown` holds the fields of [`fields::unknown`], -3 and 24.
fn assert_later_fields(unknown: &Unknown) {
    let mut keys: Vec<i128> = unknown.keys().collect();
    keys.sort_unstable();
    assert_eq!(keys, [-3, 24], "{unknown:?}");
}

/// Checks that `fixture`, with fields of later versions, decodes as `B` and as `Signed<B>` and
/// encodes back byte for byte.
fn assert_round_trip<B, F>(fixture: &F, unknown: fn(&B) -> &Unknown)
where
    B: SignedBody + PartialEq + Debug,
    F: Object,
{
    let encoded = fixture.body();
    let decoded = B::from_cbor(&encoded).unwrap_or_else(|error| panic!("{fixture:?}: {error}"));
    assert_later_fields(unknown(&decoded));
    assert_eq!(
        decoded.to_cbor(),
        encoded,
        "{fixture:?}: the body, byte for byte"
    );
    fields::assert_signed_round_trip(&encoded, &decoded);
    let signed = Signed::<B>::from_cbor(&fixture.bytes()).unwrap();
    assert_eq!(signed.body(), &decoded);
    assert_is(&signed, fixture);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_round_trips_byte_for_byte_with_its_unknown_fields() {
    for inv in invites() {
        let inv = Inv {
            extra: fields::unknown(),
            ..inv
        };
        assert_round_trip::<InviteBody, _>(&inv, |body| &body.unknown);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_acceptance_decodes_into_its_spec_fields() {
    for acc in acceptances() {
        let acceptance = Acceptance::from_cbor(&acc.bytes()).unwrap();
        let body = acceptance.body();
        assert_eq!(
            (body.invite, body.account, body.master),
            (acc.invite, acc.account, acc.master)
        );
        assert_eq!(body.handle.as_str(), acc.handle);
        assert!(body.unknown.is_empty(), "{:?}", body.unknown);
        assert_is(&acceptance, &acc);
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_acceptance_body_encodes_to_its_spec_bytes() {
    for acc in acceptances() {
        let body = acc.proto_body();
        assert_eq!(body.to_cbor(), acc.body(), "{acc:?}");
        assert_eq!(AcceptanceBody::from_cbor(&acc.body()), Ok(body), "{acc:?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_acceptance_round_trips_byte_for_byte_with_its_unknown_fields() {
    let acc = Acc {
        extra: fields::unknown(),
        ..Acc::of_newcomer()
    };
    assert_round_trip::<AcceptanceBody, _>(&acc, |body| &body.unknown);
}

/// Checks that none of `cases`, each a body and what it is, decodes alone as `B` or in a
/// `Signed<B>`.
fn assert_bodies_refused<B: SignedBody + Debug>(cases: &[(&str, Vec<u8>)]) {
    for (what, body) in cases {
        assert!(B::from_cbor(body).is_err(), "a body with {what}");
        let object = fields::signed_cbor(body, &[0x5a; 64]);
        assert!(
            Signed::<B>::from_cbor(&object).is_err(),
            "an object with {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_invite_bodies_are_refused() {
    let spec = Inv::of_member().fields();
    let (with, without) = (
        |key, value| fields::with(&spec, key, value),
        |key| fields::without(&spec, key),
    );
    let general = || bytes(GENERAL.as_bytes());
    let one = |entry: Value| Value::Array(vec![entry]);
    assert_bodies_refused::<InviteBody>(&[
        ("no invite", without(1)),
        ("no space", without(2)),
        ("no key", without(3)),
        ("no role", without(4)),
        ("no channels", without(5)),
        ("no signer", without(6)),
        ("no expiry", without(7)),
        ("an invite ID of 15 bytes", with(1, bytes(&[0x1c; 15]))),
        ("an invite ID that is text", with(1, text("invite"))),
        ("a space of 17 bytes", with(2, bytes(&[0x5a; 17]))),
        ("a key of 31 bytes", with(3, bytes(&[0x33; 31]))),
        ("a key of 33 bytes", with(3, bytes(&[0x33; 33]))),
        ("a key that is a number", with(3, Value::Uint(3))),
        ("the role 0", with(4, Value::Uint(0))),
        ("the role 3", with(4, Value::Uint(3))),
        ("a role that is text", with(4, text("member"))),
        ("channels that are no array", with(5, Value::Uint(5))),
        (
            "a channel without its history",
            with(5, one(Value::Array(vec![general()]))),
        ),
        (
            "a channel with a third item",
            with(
                5,
                one(Value::Array(vec![
                    general(),
                    Value::Bool(true),
                    Value::Bool(true),
                ])),
            ),
        ),
        (
            "a channel's history that is a number",
            with(5, one(Value::Array(vec![general(), Value::Uint(1)]))),
        ),
        (
            "a channel of 15 bytes",
            with(
                5,
                one(Value::Array(vec![bytes(&[0xc1; 15]), Value::Bool(true)])),
            ),
        ),
        (
            "a channel that is a map",
            with(5, one(Value::Map(vec![(1, general())]))),
        ),
        ("a signer of 17 bytes", with(6, bytes(&[0x11; 17]))),
        ("an expiry that is text", with(7, text("tomorrow"))),
        ("an expiry that is a boolean", with(7, Value::Bool(true))),
        (
            "a body that is an array",
            Value::Array(vec![general()]).encode(),
        ),
    ]);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_acceptance_bodies_are_refused() {
    let spec = Acc::of_newcomer().fields();
    let (with, without) = (
        |key, value| fields::with(&spec, key, value),
        |key| fields::without(&spec, key),
    );
    assert_bodies_refused::<AcceptanceBody>(&[
        ("no invite", without(1)),
        ("no account", without(2)),
        ("no master key", without(3)),
        ("no handle", without(4)),
        ("an invite ID of 17 bytes", with(1, bytes(&[0x1c; 17]))),
        ("an account of 15 bytes", with(2, bytes(&[0xad; 15]))),
        ("an account that is text", with(2, text("bob"))),
        ("a master key of 31 bytes", with(3, bytes(&[0x02; 31]))),
        ("a master key of 33 bytes", with(3, bytes(&[0x02; 33]))),
        ("a handle that is bytes", with(4, bytes(b"bob"))),
        ("an empty handle", with(4, text(""))),
        ("a handle in uppercase", with(4, text("Bob"))),
        ("a handle with a space", with(4, text("bob smith"))),
        ("a handle that starts with '-'", with(4, text("-bob"))),
        (
            "a body that is an array",
            Value::Array(vec![text("bob")]).encode(),
        ),
    ]);
}

/// Checks that no prefix of `fixture`'s object or body, nor either with a byte after it, decodes,
/// and no object of its body and a signature of another form.
fn assert_truncated_and_malformed_refused<B: SignedBody + Debug>(fixture: &impl Object) {
    for input in fields::truncated_and_longer(&fixture.bytes()) {
        assert!(Signed::<B>::from_cbor(&input).is_err(), "{input:?}");
    }
    for input in fields::truncated_and_longer(&fixture.body()) {
        assert!(B::from_cbor(&input).is_err(), "{input:?}");
    }
    let (body, signature) = (bytes(&fixture.body()), bytes(&fixture.signature()));
    for (what, items) in [
        (
            "a signature of 63 bytes",
            vec![body.clone(), bytes(&[0x5a; 63])],
        ),
        (
            "a signature of 65 bytes",
            vec![body.clone(), bytes(&[0x5a; 65])],
        ),
        (
            "a body that is a map",
            vec![fixture.value(), signature.clone()],
        ),
        ("no signature", vec![body.clone()]),
        ("a third item", vec![body, signature.clone(), signature]),
    ] {
        let input = Value::Array(items).encode();
        assert!(
            Signed::<B>::from_cbor(&input).is_err(),
            "{fixture:?} with {what}"
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_invites_are_refused() {
    assert_truncated_and_malformed_refused::<InviteBody>(&Inv::of_member());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn malformed_and_truncated_acceptances_are_refused() {
    assert_truncated_and_malformed_refused::<AcceptanceBody>(&Acc::of_newcomer());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_invite_is_no_acceptance_and_an_acceptance_no_invite() {
    assert!(Acceptance::from_cbor(&Inv::of_member().bytes()).is_err());
    assert!(Invite::from_cbor(&Acc::of_newcomer().bytes()).is_err());
}
