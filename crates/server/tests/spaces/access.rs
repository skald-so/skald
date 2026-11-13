//! Who gets a space: sessions of the accounts that are members in its latest revision, and its
//! creator's before revision 1. They get its revisions as stored and the devices of the accounts
//! its revisions list, each with its certificate as the device sent it; the account's spaces are
//! the ones it gets.

use skald_proto::{Role, SpaceId};

use crate::api::{ACME, Person, Session, Team, assert_refused};
use crate::common::device_flow::text;
use crate::common::devices::b64;
use crate::rosters::{member, next};

/// Checks that `person`'s session gets `space` with the revisions `stored`.
fn gets(person: &Person, space: SpaceId, stored: &[Vec<u8>]) {
    let spaces = person.session.spaces();
    let who = &person.handle;
    assert_eq!(
        spaces.get(&space.to_string()),
        Some(&ACME.to_owned()),
        "{who} lists {ACME}: {spaces:?}"
    );
    assert_eq!(person.session.revisions(space), stored, "{who}");
}

/// Checks that `person`'s session gets nothing of `space`, whose revisions are `stored`.
fn gets_nothing(person: &Person, space: SpaceId, stored: &[Vec<u8>]) {
    let who = &person.handle;
    let spaces = person.session.spaces();
    assert!(
        !spaces.contains_key(&space.to_string()),
        "{who} lists {space}: {spaces:?}"
    );
    let answers = [
        ("revisions", person.session.revisions_response(space)),
        ("devices", person.session.devices_response(space)),
        ("a channel ID", person.session.channel_response(space)),
    ];
    for (what, answer) in answers {
        assert_refused(&answer, &format!("{who}: {what} of {space}"));
        let body = text(&answer);
        for revision in stored {
            assert!(!body.contains(&b64(revision)), "{who}: {what}: {body}");
        }
    }
}

#[test]
fn the_creator_gets_the_space_before_its_first_revision_and_no_one_else_does() {
    let team = Team::new(3_120_850_001);
    gets(&team.alice, team.space, &[]);
    for person in [&team.bob, &team.dave] {
        gets_nothing(person, team.space, &[]);
    }
}

#[test]
fn only_sessions_of_the_latest_revisions_members_get_the_space() {
    let team = Team::new(3_120_850_011);
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);
    let mut stored = team.with_members().to_vec();
    for person in [alice, bob, carol] {
        gets(person, team.space, &stored);
    }
    gets_nothing(dave, team.space, &stored);

    // Revision 3 removes carol; revision 4 takes dave in.
    let mut body = next(&stored[1], &alice.laptop);
    body.members = skald_proto::NonEmpty::from_vec(
        body.members
            .iter()
            .filter(|member| member.account != carol.account)
            .cloned()
            .collect(),
    )
    .expect("a member");
    let third = alice.sign(&body);
    team.stored_by(alice, &third);
    stored.push(third);
    gets_nothing(carol, team.space, &stored);
    gets_nothing(dave, team.space, &stored);

    let mut body = next(&stored[2], &bob.laptop);
    body.members.push(member(dave, Role::Member));
    let fourth = bob.sign(&body);
    team.stored_by(bob, &fourth);
    stored.push(fourth);
    for person in [alice, bob, dave] {
        gets(person, team.space, &stored);
    }
    gets_nothing(carol, team.space, &stored);
}

#[test]
fn an_unknown_space_or_no_session_gets_nothing() {
    let team = Team::new(3_120_850_021);
    let stored = team.with_members().to_vec();
    let unknown = SpaceId::generate();
    assert_refused(&team.alice.session.revisions_response(unknown), "revisions");
    assert_refused(&team.alice.session.devices_response(unknown), "devices");
    assert_refused(&team.alice.session.channel_response(unknown), "channel");
    let revision = team.alice.sign(&next(&stored[1], &team.alice.laptop));
    assert_refused(&team.alice.session.store(unknown, &revision), "store");

    let none = Session::none(&team.flow);
    let answers = [
        none.revisions_response(team.space),
        none.devices_response(team.space),
        none.store(team.space, &revision),
    ];
    for answer in answers {
        assert_eq!(answer.status, 401, "{}", text(&answer));
    }
    assert_eq!(team.alice.session.revisions(team.space), stored);
}

#[test]
fn a_spaces_devices_are_those_of_the_accounts_its_revisions_list_with_their_certificates() {
    let team = Team::new(3_120_850_031);
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);
    let stored = team.with_members();

    // Revision 3 removes carol, whom revision 2 still lists.
    let mut body = next(&stored[1], &alice.laptop);
    body.members = skald_proto::NonEmpty::new(member(alice, Role::Admin));
    body.members.push(member(bob, Role::Admin));
    team.stored_by(alice, &alice.sign(&body));

    for person in [alice, bob] {
        let devices = person.session.devices(team.space);
        for listed in [alice, bob, carol] {
            assert_eq!(
                devices.get(&listed.laptop.id.to_string()),
                Some(&Some(listed.cert.clone())),
                "{}: {}'s laptop in {devices:?}",
                person.handle,
                listed.handle
            );
        }
        for device in [&dave.laptop, &dave.uncertified] {
            assert!(
                !devices.contains_key(&device.id.to_string()),
                "{}: dave's device in {devices:?}",
                person.handle
            );
        }
    }
}
