//! A space's roster and policy: `docs/protocol/membership.md`, "Roster" and "Policy", encoded as
//! "Signed objects" and `docs/protocol/README.md`, "Encoding", say. The acceptance tests of the
//! roster are in `crates/crypto/tests/rosters/`.

mod common;

use common::{assert_field_problem, hex};
use skald_proto::{
    AccountId, Cbor, DeviceId, FieldProblem, Mode, Role, Roster, RosterBody, SpaceId,
};

/// `n` copies of the hex byte `byte`.
fn fill(byte: &str, n: usize) -> String {
    byte.repeat(n)
}

/// A revision's body with every optional field and fields of later versions: revision 2 of
/// `acme`, the admin alice and the member carol, the service agent `ci/deploy-bot`, one channel and
/// the ceilings `[read-only, 9]`. `history` is the policy's, in hex.
fn body_hex(history: &str, alice_in_channel: &str) -> String {
    format!(
        "ab
         01 50{space}
         02 64 61636d65
         03 02
         04 5820{prev}
         05 50{signer}
         06 1a 6911a780
         07 82
            a4 01 50{alice} 02 65 616c696365 03 01 04 5820{alice_master}
            a7 01 50{carol} 02 65 6361726f6c 03 02 04 5820{carol_master}
               05 50{invite} 06 81 50{excluded} 07 65 6c61746572
         08 81 a3 01 50{ci} 02 62 6369 03 6a 6465706c6f792d626f74
         09 81 a3 01 50{channel} 02 82 82 50{alice} {alice_in_channel} 82 50{carol} f4 03 81 50{ci}
         0a a3 01 82 01 09 02 {history} 03 f5
         0b 65 6c61746572",
        space = fill("5a", 16),
        prev = fill("77", 32),
        signer = fill("11", 16),
        alice = fill("a1", 16),
        alice_master = fill("01", 32),
        carol = fill("ca", 16),
        carol_master = fill("03", 32),
        invite = fill("1c", 16),
        excluded = fill("22", 16),
        ci = fill("21", 16),
        channel = fill("c1", 16),
    )
}

/// The body of [`body_hex`] with a history of 30 days, alice reading the channel's history.
fn body() -> Vec<u8> {
    hex(&body_hex("1a 00278d00", "f5"))
}

/// The roster of [`body`], its signature 64 bytes of `0x55`.
fn roster() -> Vec<u8> {
    hex(&format!(
        "82 59018c {} 5840{}",
        hex_of(&body()),
        fill("55", 64)
    ))
}

fn hex_of(bytes: &[u8]) -> String {
    common::to_hex(bytes)
}

#[test]
fn a_roster_decodes_into_its_fields_and_encodes_back() {
    let encoded = roster();
    let roster = Roster::from_cbor(&encoded).unwrap();
    assert_eq!(roster.body_bytes(), body().as_slice());
    assert_eq!(roster.signature(), &[0x55; 64]);
    assert_eq!(roster.to_cbor(), encoded);
    assert_eq!(roster.body().to_cbor(), body());

    let body = roster.body();
    assert_eq!(body.space, SpaceId::from_bytes([0x5a; 16]));
    assert_eq!(body.name.as_str(), "acme");
    assert_eq!((body.revision, body.prev), (2, Some([0x77; 32])));
    assert_eq!(body.signer, DeviceId::from_bytes([0x11; 16]));
    assert_eq!(body.timestamp, 0x6911_a780);
    let [alice, carol] = [&body.members[0], &body.members[1]];
    assert_eq!(alice.account, AccountId::from_bytes([0xa1; 16]));
    assert_eq!(
        (alice.handle.as_str(), alice.role, alice.master),
        ("alice", Role::Admin, [0x01; 32])
    );
    assert_eq!((alice.invite, &alice.excluded), (None, &None));
    assert_eq!(
        (carol.handle.as_str(), carol.role, carol.master),
        ("carol", Role::Member, [0x03; 32])
    );
    assert_eq!(carol.invite.map(|id| *id.as_bytes()), Some([0x1c; 16]));
    assert_eq!(
        carol.excluded.as_deref(),
        Some([DeviceId::from_bytes([0x22; 16])].as_slice())
    );
    assert_eq!(carol.unknown.keys().collect::<Vec<_>>(), [7]);

    let [ci] = body.service_agents.as_slice() else {
        panic!("not one service agent: {:?}", body.service_agents)
    };
    assert_eq!(ci.device, DeviceId::from_bytes([0x21; 16]));
    assert_eq!((ci.owner.as_str(), ci.agent.as_str()), ("ci", "deploy-bot"));

    let [channel] = body.channels.as_slice() else {
        panic!("not one channel: {:?}", body.channels)
    };
    let members: Vec<_> = channel
        .members
        .iter()
        .map(|member| (member.account, member.history))
        .collect();
    assert_eq!(
        members,
        [
            (AccountId::from_bytes([0xa1; 16]), true),
            (AccountId::from_bytes([0xca; 16]), false)
        ]
    );
    assert_eq!(channel.services, [DeviceId::from_bytes([0x21; 16])]);

    assert_eq!(body.policy.ceilings.member, Mode::ReadOnly);
    assert_eq!(body.policy.ceilings.service, Mode::Later(9));
    assert_eq!(body.policy.ceilings.service.effective(), Mode::ReadOnly);
    assert_eq!(Mode::Full.effective(), Mode::Full);
    assert_eq!(body.policy.history, 30 * 86_400);
    assert_eq!(body.policy.unknown.keys().collect::<Vec<_>>(), [3]);
    assert_eq!(body.unknown.keys().collect::<Vec<_>>(), [11]);
}

#[test]
fn a_revision_hash_is_sha_256_of_the_encoded_roster() {
    let roster = Roster::from_cbor(&roster()).unwrap();
    // SHA-256 of `roster()`, computed apart from the crate.
    assert_eq!(
        hex_of(&roster.hash()),
        "3c2c8764806197dda38b37ea007abdbf4d6b1f64fe0228784b6ed1971493e7fe"
    );
}

#[test]
fn a_policy_history_is_at_least_a_day() {
    let day = RosterBody::from_cbor(&hex(&body_hex("1a 00015180", "f5"))).unwrap();
    assert_eq!(day.policy.history, 86_400);
    assert_field_problem(
        RosterBody::from_cbor(&hex(&body_hex("1a 0001517f", "f5"))),
        &FieldProblem::Below {
            min: 86_400,
            actual: 86_399,
        },
    );
}

#[test]
fn a_channel_members_history_is_a_boolean() {
    let body = RosterBody::from_cbor(&hex(&body_hex("1a 00278d00", "f4"))).unwrap();
    assert!(!body.channels[0].members[0].history);
    for value in ["01", "f6", "60"] {
        assert_field_problem(
            RosterBody::from_cbor(&hex(&body_hex("1a 00278d00", value))),
            &FieldProblem::Type("a boolean"),
        );
    }
}

#[test]
fn revision_one_has_its_creator_as_the_only_admin_and_a_new_spaces_policy() {
    let body = RosterBody::first(
        SpaceId::from_bytes([0x5a; 16]),
        "acme".parse().unwrap(),
        AccountId::from_bytes([0xa1; 16]),
        "alice".parse().unwrap(),
        [0x01; 32],
        DeviceId::from_bytes([0x11; 16]),
        0x6911_a780,
    );
    assert_eq!(
        hex_of(&body.to_cbor()),
        hex_of(&hex(&format!(
            "a9 01 50{space} 02 64 61636d65 03 01 05 50{signer} 06 1a 6911a780
             07 81 a4 01 50{alice} 02 65 616c696365 03 01 04 5820{master}
             08 80 09 80 0a a2 01 82 01 01 02 1a 00278d00",
            space = fill("5a", 16),
            signer = fill("11", 16),
            alice = fill("a1", 16),
            master = fill("01", 32),
        )))
    );
}

#[test]
fn member_is_the_first_entry_of_an_account_and_none_for_another() {
    let (alice, carol, bob) = (
        AccountId::from_bytes([0xa1; 16]),
        AccountId::from_bytes([0xca; 16]),
        AccountId::from_bytes([0xb0; 16]),
    );
    let mut body = RosterBody::first(
        SpaceId::from_bytes([0x5a; 16]),
        "acme".parse().unwrap(),
        alice,
        "alice".parse().unwrap(),
        [0x01; 32],
        DeviceId::from_bytes([0x11; 16]),
        0x6911_a780,
    );
    let mut member = body.members[0].clone();
    member.account = carol;
    member.handle = "carol".parse().unwrap();
    member.role = Role::Member;
    body.members.push(member.clone());
    // A repeated account, as a revision that fails check 3 lists it.
    member.account = alice;
    member.handle = "alice-again".parse().unwrap();
    body.members.push(member);

    assert_eq!(
        body.member(carol).map(|member| member.handle.as_str()),
        Some("carol")
    );
    let found = body.member(alice).expect("alice is a member");
    assert_eq!(found.handle.as_str(), "alice", "not the first entry");
    assert_eq!(found.role, Role::Admin);
    assert_eq!(body.member(bob), None);
}
