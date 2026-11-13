//! The answers of refused requests: a store from an account that does not get the space gets the
//! answer of an unknown space, one from a member that is no admin gets `403`, and a creation of a
//! space that has a revision gets `409`.

use serde_json::Value;
use skald_proto::{Role, SpaceId};

use crate::api::{ACME, Person, Team};
use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::rosters::{member, next};

/// The status and body of `response`, a refusal with a problem details object.
fn problem(response: &Response) -> (u16, Value) {
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{}: {}",
        response.content_type,
        text(response)
    );
    (response.status, json(response))
}

/// Checks that the server answers `revisions`, each with what it is, from `person`'s session as it
/// answers a store into a space that does not exist, and keeps the revisions `stored`.
fn hidden(team: &Team, person: &Person, revisions: &[(&str, Vec<u8>)], stored: &[Vec<u8>]) {
    let unknown = problem(&person.session.store(SpaceId::generate(), &revisions[0].1));
    assert_eq!(unknown.0, 404, "an unknown space: {}", unknown.1);
    for (what, revision) in revisions {
        let answer = problem(&person.session.store(team.space, revision));
        assert_eq!(answer, unknown, "{}: {what}", person.handle);
    }
    assert_eq!(team.bob.session.revisions(team.space), stored);
}

#[test]
fn a_store_from_an_account_outside_the_space_gets_the_answer_of_an_unknown_space() {
    let team = Team::new(3_120_870_001);
    let (alice, dave) = (&team.alice, &team.dave);
    let stored = team.with_members().to_vec();

    let mut own = next(&stored[1], &dave.laptop);
    own.members.push(member(dave, Role::Admin));
    let mut taken_number = next(&stored[0], &dave.laptop);
    taken_number.members.push(member(dave, Role::Admin));
    let revisions = [
        ("its own revision 3", dave.sign(&own)),
        (
            "a copy of an admin's revision 3",
            alice.sign(&next(&stored[1], &alice.laptop)),
        ),
        ("a copy of stored revision 2", stored[1].clone()),
        ("another revision 2", dave.sign(&taken_number)),
        ("bytes that do not decode", vec![0x82, 0x41]),
    ];
    hidden(&team, dave, &revisions, &stored);
}

#[test]
fn an_admin_that_took_its_account_out_gets_the_answer_of_an_unknown_space_but_for_its_retry() {
    let team = Team::new(3_120_870_011);
    let alice = &team.alice;
    let stored = team.alice_left();

    let revisions = [
        (
            "another revision 3",
            alice.sign(&next(&stored[1], &alice.laptop)),
        ),
        (
            "another revision 2",
            alice.sign(&next(&stored[0], &alice.laptop)),
        ),
        ("revision 4", alice.sign(&next(&stored[2], &alice.laptop))),
        ("bytes that do not decode", vec![0x82, 0x41]),
    ];
    hidden(&team, alice, &revisions, &stored);
}

#[test]
fn a_store_from_a_member_that_is_no_admin_is_forbidden() {
    let team = Team::new(3_120_870_021);
    let (alice, carol) = (&team.alice, &team.carol);
    let stored = team.with_members().to_vec();

    let revisions = [
        (
            "its own revision 3",
            carol.sign(&next(&stored[1], &carol.laptop)),
        ),
        (
            "a copy of an admin's revision 3",
            alice.sign(&next(&stored[1], &alice.laptop)),
        ),
    ];
    for (what, revision) in revisions {
        let (status, body) = problem(&carol.session.store(team.space, &revision));
        assert_eq!(status, 403, "{what}: {body}");
    }
    assert_eq!(alice.session.revisions(team.space), stored);
}

#[test]
fn the_creators_same_request_is_refused_once_its_space_has_a_revision() {
    let team = Team::new(3_120_870_031);
    let alice = &team.alice;
    team.with_members();

    let (status, body) = problem(&alice.session.create(ACME));
    assert_eq!(status, 409, "{body}");
    let spaces = alice.session.spaces();
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    assert_eq!(spaces.get(&team.space.to_string()), Some(&ACME.to_owned()));
}
