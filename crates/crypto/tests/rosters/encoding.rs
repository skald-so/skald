//! The roster and policy types of `skald-proto`: `membership.md`, "Roster" and "Policy", encoded as
//! "Signed objects" and `README.md`, "Encoding", say. A revision keeps its body's bytes as it got
//! them. Malformed and truncated input is an error, never a panic.

use skald_proto::{Cbor, DecodeError, Mode, Role, Roster, RosterBody};

use crate::cbor::{Value, bytes, text};
use crate::fixtures::*;

/// `current` with every optional field set, and fields of later versions in the body, a member
/// and the policy.
fn rich() -> Rev {
    let mut rev = current();
    rev.members[2].excluded = vec![device(CAROL_LAPTOP)];
    rev.members[1].extra = vec![(7, text("later")), (-1, Value::Uint(3))];
    rev.extra = vec![(11, bytes(b"later")), (99, Value::Array(vec![]))];
    rev.policy_extra = vec![(3, Value::Bool(true))];
    rev
}

/// The encoded roster of an encoded `body`, signed by alice's laptop.
fn roster_of(body: &[u8]) -> Vec<u8> {
    let signature = signature(ALICE_LAPTOP, "skald roster", body);
    Value::Array(vec![bytes(body), bytes(&signature)]).encode()
}

/// `value`, a map, with `key` set to `field`, or left out if `field` is `None`.
fn with(value: &Value, key: i64, field: Option<Value>) -> Value {
    let Value::Map(fields) = value else {
        panic!("not a map: {value:?}")
    };
    let mut fields: Vec<_> = fields.iter().filter(|(k, _)| *k != key).cloned().collect();
    fields.extend(field.map(|field| (key, field)));
    Value::Map(fields)
}

/// `rev` with the first member's field `key` set to `field`, or left out if it is `None`.
fn with_member_field(rev: &Rev, key: i64, field: Option<Value>) -> Value {
    let mut members: Vec<Value> = rev.members.iter().map(Member::value).collect();
    members[0] = with(&members[0], key, field);
    with(&rev.value(), 7, Some(Value::Array(members)))
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_decodes_into_its_spec_fields() {
    let rev = rich();
    let roster = Roster::from_cbor(&rev.bytes()).unwrap();
    let body = roster.body();

    assert_eq!(body.space, SPACE);
    assert_eq!(body.name.as_str(), "acme");
    assert_eq!(body.revision, 2);
    assert_eq!(body.prev, Some([0x77; 32]));
    assert_eq!(body.signer, device(ALICE_LAPTOP));
    assert_eq!(body.timestamp, TIMESTAMP);

    assert_eq!(body.members.len(), 3);
    let [alice, bob, carol] = [&body.members[0], &body.members[1], &body.members[2]];
    assert_eq!(alice.account, ALICE);
    assert_eq!(alice.handle.as_str(), "alice");
    assert_eq!(alice.role, Role::Admin);
    assert_eq!(alice.master, public(ALICE_MASTER));
    assert_eq!(alice.invite.map(|invite| *invite.as_bytes()), None);
    assert_eq!(alice.excluded.as_deref(), None);
    assert_eq!(bob.role, Role::Admin);
    assert_eq!(
        bob.invite.map(|invite| *invite.as_bytes()),
        Some(BOB_INVITE)
    );
    assert_eq!(carol.role, Role::Member);
    assert_eq!(carol.master, public(CAROL_MASTER));
    assert_eq!(
        carol.excluded.as_deref(),
        Some([device(CAROL_LAPTOP)].as_slice())
    );

    assert_eq!(body.service_agents.len(), 1);
    let ci = &body.service_agents[0];
    assert_eq!(ci.device, device(BOB_CI));
    assert_eq!(ci.owner.as_str(), "ci");
    assert_eq!(ci.agent.as_str(), "deploy-bot");

    assert_eq!(body.channels.len(), 1);
    let channel = &body.channels[0];
    assert_eq!(channel.id, CHANNEL);
    let members: Vec<_> = channel
        .members
        .iter()
        .map(|member| (member.account, member.history))
        .collect();
    assert_eq!(members, [(ALICE, true), (CAROL, false)]);
    assert_eq!(channel.services.as_slice(), [device(BOB_CI)].as_slice());

    assert_eq!(body.policy.ceilings.member, Mode::ReadOnly);
    assert_eq!(body.policy.ceilings.service, Mode::Full);
    assert_eq!(body.policy.history, THIRTY_DAYS);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_round_trips_byte_for_byte_with_its_unknown_fields() {
    let rev = rich();
    let roster = Roster::from_cbor(&rev.bytes()).unwrap();
    assert_holds(&roster, &rev);

    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    assert_eq!(body.unknown.keys().collect::<Vec<_>>(), [11, 99]);
    assert_eq!(body.members[1].unknown.keys().collect::<Vec<_>>(), [7, -1]);
    assert_eq!(body.policy.unknown.keys().collect::<Vec<_>>(), [3]);
    assert_eq!(body.to_cbor(), rev.body());
    assert_eq!(&body, roster.body());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_without_optional_fields_round_trips() {
    let mut rev = current();
    rev.members = vec![alice()];
    rev.service_agents.clear();
    rev.channels.clear();
    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    assert!(body.service_agents.is_empty());
    assert!(body.channels.is_empty());
    assert_eq!(
        body.members[0].invite.map(|invite| *invite.as_bytes()),
        None
    );
    assert_eq!(body.to_cbor(), rev.body());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_one_has_no_prev() {
    let mut rev = current();
    rev.revision = 1;
    rev.prev = None;
    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    assert_eq!((body.revision, body.prev), (1, None));
    assert_eq!(body.to_cbor(), rev.body());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_mode_this_version_does_not_know_reads_as_read_only_and_round_trips() {
    let mut rev = current();
    rev.ceilings = [9, FULL];
    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    assert_eq!(body.policy.ceilings.member.effective(), Mode::ReadOnly);
    assert_eq!(body.policy.ceilings.service.effective(), Mode::Full);
    assert_eq!(Mode::ReadOnly.effective(), Mode::ReadOnly);
    assert_eq!(body.to_cbor(), rev.body());

    rev.ceilings = [FULL, 300];
    let body = RosterBody::from_cbor(&rev.body()).unwrap();
    assert_eq!(body.policy.ceilings.member.effective(), Mode::Full);
    assert_eq!(body.policy.ceilings.service.effective(), Mode::ReadOnly);
    assert_eq!(body.to_cbor(), rev.body());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn revision_one_of_a_new_space_has_its_creator_as_the_only_admin() {
    let body = first_body();

    assert_eq!(body.space, SPACE);
    assert_eq!(body.name.as_str(), "acme");
    assert_eq!((body.revision, body.prev), (1, None));
    assert_eq!(body.signer, device(ALICE_LAPTOP));
    assert_eq!(body.members.len(), 1);
    assert_eq!(body.members[0].account, ALICE);
    assert_eq!(body.members[0].handle.as_str(), "alice");
    assert_eq!(body.members[0].role, Role::Admin);
    assert_eq!(body.members[0].master, public(ALICE_MASTER));
    assert!(body.service_agents.is_empty());
    assert!(body.channels.is_empty());
    // A new space's policy: `[read-only, read-only]` and 30 days.
    assert_eq!(body.policy.ceilings.member, Mode::ReadOnly);
    assert_eq!(body.policy.ceilings.service, Mode::ReadOnly);
    assert_eq!(body.policy.history, THIRTY_DAYS);

    assert_eq!(body.to_cbor(), revision_one().body());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_hash_is_sha_256_of_the_encoded_roster() {
    let roster = Roster::from_cbor(&revision_one().bytes()).unwrap();
    // SHA-256 of `revision_one().bytes()`, computed apart from the crates under test.
    assert_eq!(roster.hash(), hex32(REVISION_ONE_HASH));
    assert_ne!(roster.hash(), current().roster().hash());
}

/// SHA-256 of the encoded roster of `a_revision_hash_is_sha_256_of_the_encoded_roster`.
const REVISION_ONE_HASH: &str = "1969c9b45474e392e29e226b911babede6bb70fd10f40395ee3d4e3179f21433";

fn hex32(hex: &str) -> [u8; 32] {
    let mut out = [0; 32];
    for (at, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * at..2 * at + 2], 16).unwrap();
    }
    out
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_truncation_of_a_revision_is_rejected() {
    let encoded = rich().bytes();
    for len in 0..encoded.len() {
        assert_eq!(
            Roster::from_cbor(&encoded[..len]),
            Err(DecodeError::Truncated),
            "{len} of {} bytes",
            encoded.len()
        );
    }
    let body = rich().body();
    for len in 0..body.len() {
        assert!(
            Roster::from_cbor(&roster_of(&body[..len])).is_err(),
            "a body of {len} of {} bytes",
            body.len()
        );
        assert_eq!(
            RosterBody::from_cbor(&body[..len]),
            Err(DecodeError::Truncated),
            "{len} of {} bytes",
            body.len()
        );
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn every_corruption_of_a_revision_decodes_or_fails_without_a_panic() {
    let encoded = rich().bytes();
    for at in 0..encoded.len() {
        for flip in [0x01, 0x20, 0x80, 0xff] {
            let mut corrupted = encoded.clone();
            corrupted[at] ^= flip;
            let _ = Roster::from_cbor(&corrupted);
        }
    }
    for garbage in [
        &[0xff][..],
        &[0x82, 0x40],
        &[0x9f, 0xff],
        &[0x82, 0x41, 0xa0, 0x40],
    ] {
        assert!(Roster::from_cbor(garbage).is_err(), "{garbage:02x?}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_that_breaks_the_encoding_rules_is_rejected() {
    let encoded = current().bytes();

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(
        Roster::from_cbor(&trailing),
        Err(DecodeError::TrailingBytes(1))
    );

    // The body's map with its first two keys swapped: 2 before 1.
    let body = current().body();
    let Value::Map(fields) = current().value() else {
        unreachable!()
    };
    let field = |key: i64| {
        fields
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, value)| value.encode())
            .unwrap()
    };
    let mut unsorted = vec![body[0], 0x02];
    unsorted.extend(field(2));
    unsorted.push(0x01);
    unsorted.extend(field(1));
    unsorted.extend(&body[1 + 1 + field(1).len() + 1 + field(2).len()..]);
    assert_eq!(unsorted.len(), body.len());
    assert!(RosterBody::from_cbor(&unsorted).is_err());
    assert!(Roster::from_cbor(&roster_of(&unsorted)).is_err());

    // A repeated key: `name` twice, in place of `space`.
    let mut repeated = vec![body[0], 0x02];
    repeated.extend(field(2));
    repeated.push(0x02);
    repeated.extend(field(2));
    repeated.extend(&body[1 + 1 + field(1).len() + 1 + field(2).len()..]);
    assert!(RosterBody::from_cbor(&repeated).is_err());

    // The revision number in a head longer than its shortest form: keys 1 and 2 come first.
    let mut long = current();
    long.revision = 23;
    let shortest = long.body();
    let at = 1 + 1 + field(1).len() + 1 + field(2).len();
    assert_eq!(shortest[at..at + 2], [0x03, 0x17]);
    let mut not_shortest = shortest[..at + 1].to_vec();
    not_shortest.extend([0x18, 0x17]);
    not_shortest.extend(&shortest[at + 2..]);
    assert!(RosterBody::from_cbor(&not_shortest).is_err());
    assert!(Roster::from_cbor(&roster_of(&not_shortest)).is_err());
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_that_breaks_the_cddl_does_not_decode() {
    let rev = current();
    let body = rev.value();
    let mut broken: Vec<(String, Value)> = Vec::new();
    for key in [1, 2, 3, 5, 6, 7, 8, 9, 10] {
        broken.push((format!("without field {key}"), with(&body, key, None)));
    }
    broken.extend([
        (
            "a 15-byte space".to_owned(),
            with(&body, 1, Some(bytes(&[0x5a; 15]))),
        ),
        (
            "a name of bytes".to_owned(),
            with(&body, 2, Some(bytes(b"acme"))),
        ),
        (
            "a revision of text".to_owned(),
            with(&body, 3, Some(text("2"))),
        ),
        (
            "revision 0".to_owned(),
            with(&body, 3, Some(Value::Uint(0))),
        ),
        (
            "a 31-byte prev".to_owned(),
            with(&body, 4, Some(bytes(&[0x77; 31]))),
        ),
        (
            "a 17-byte signer".to_owned(),
            with(&body, 5, Some(bytes(&[1; 17]))),
        ),
        (
            "members not an array".to_owned(),
            with(&body, 7, Some(Value::Map(vec![]))),
        ),
        (
            "a policy not a map".to_owned(),
            with(&body, 10, Some(Value::Array(vec![]))),
        ),
        (
            "a policy without history".to_owned(),
            with(&body, 10, Some(with(&rev.policy(), 2, None))),
        ),
        (
            "a policy without ceilings".to_owned(),
            with(&body, 10, Some(with(&rev.policy(), 1, None))),
        ),
        (
            "ceilings of three modes".to_owned(),
            with(
                &body,
                10,
                Some(with(
                    &rev.policy(),
                    1,
                    Some(Value::Array(vec![Value::Uint(1); 3])),
                )),
            ),
        ),
        (
            "a member without master".to_owned(),
            with_member_field(&rev, 4, None),
        ),
        (
            "a member with a 31-byte master".to_owned(),
            with_member_field(&rev, 4, Some(bytes(&[1; 31]))),
        ),
        (
            "a member without role".to_owned(),
            with_member_field(&rev, 3, None),
        ),
        (
            "a member's handle of bytes".to_owned(),
            with_member_field(&rev, 2, Some(bytes(b"alice"))),
        ),
        (
            "an empty excluded list".to_owned(),
            with_member_field(&rev, 6, Some(Value::Array(vec![]))),
        ),
    ]);
    for (what, body) in broken {
        let body = body.encode();
        assert!(RosterBody::from_cbor(&body).is_err(), "{what}");
        assert!(Roster::from_cbor(&roster_of(&body)).is_err(), "{what}");
    }

    let signature = signature(ALICE_LAPTOP, "skald roster", &rev.body());
    let short_signature = Value::Array(vec![bytes(&rev.body()), bytes(&signature[..63])]);
    let three_items = Value::Array(vec![
        bytes(&rev.body()),
        bytes(&signature),
        bytes(&signature),
    ]);
    let body_as_map = Value::Array(vec![rev.value(), bytes(&signature)]);
    for (what, roster) in [
        ("a 63-byte signature", short_signature),
        ("three items", three_items),
        ("a body not in a byte string", body_as_map),
    ] {
        assert!(Roster::from_cbor(&roster.encode()).is_err(), "{what}");
    }
}

/// A revision that breaks the CDDL in a way a decoder or check 3 may catch: the client never
/// accepts it.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_revision_with_no_member_or_an_undefined_role_is_never_accepted() {
    let mut no_member = next();
    no_member.members.clear();
    no_member.channels.clear();
    no_member.service_agents.clear();
    assert!(accepts_bytes(&no_member.bytes()).is_err());

    let mut role_three = next();
    role_three.members[2].role = 3;
    assert!(accepts_bytes(&role_three.bytes()).is_err());
}
