//! What a shared space keeps of an erased account, its members still get: its revisions, its
//! devices' certificates and revocations, the entries its devices wrote and the mark that it was
//! deleted. A space it created counts against no account's owned spaces.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use skald_proto::SpaceId;

use crate::api::Person;
use crate::common::device_flow::{json, text};
use crate::deletion::{bearer, deleted_accounts, deletes, members};
use crate::erasure::{ERASED_AFTER, Kept, Traces, erased_within, move_deletion_back};
use crate::fixture::Fixture;
use crate::sessions::Api;
use crate::shared::{Shared, alices_records, certified};

/// The account's owned spaces and devices: `GET`.
const USAGE: &str = "/account/usage";

/// In the shared scenario, once alice's account is erased, bob's and carol's sessions still get
/// `acme` with the same revisions, the certificates of every device they got before, alice's
/// laptop's and phone's among them, her revocation, her entry in `general`, and her account marked
/// deleted among the space's members and devices.
#[test]
fn members_still_get_its_revisions_certificates_revocations_entries_and_deletion_mark() {
    let shared = Shared::new(3_304_020_001, true);
    let t = &shared.f().team;
    let alice = t.alice.account.to_string();
    for device in [t.alice.laptop.id, shared.phone.id] {
        assert!(
            shared.certs.contains_key(&device.to_string()),
            "{device}'s certificate before"
        );
    }

    shared.deletes_long_ago();
    erased_within(shared.db(), &shared.alice, &shared.kept);

    for person in [&t.bob, &t.carol] {
        let session = &person.session;
        let who = &person.handle;
        assert!(
            session.spaces().contains_key(&t.space.to_string()),
            "{who}: acme listed"
        );
        assert_eq!(
            session.revisions(t.space),
            shared.revisions,
            "{who}: revisions"
        );
        assert_eq!(
            certified(person, t.space),
            shared.certs,
            "{who}: certificates"
        );
        assert!(
            session
                .space_revocations(t.space)
                .contains(&shared.revocation),
            "{who}: alice's revocation"
        );
        let deleted = deleted_accounts(&session.devices_response(t.space));
        assert_eq!(deleted, BTreeSet::from([alice.clone()]), "{who}: deleted");
        let marked = members(session, t.space);
        assert!(
            marked.contains(&("alice".to_owned(), true)),
            "{who}: alice marked deleted in {marked:?}"
        );
        assert!(
            shared
                .entries_for(person)
                .contains(&shared.entry.bytes().to_vec()),
            "{who}: alice's entry"
        );
    }
}

/// The spaces `person` owns as `GET /account/usage` gives them: each one's name by its ID.
#[track_caller]
fn owned_by(api: &Api, person: &Person) -> BTreeMap<String, String> {
    let authorization = bearer(person.session.token());
    let response = api.call("get", USAGE, USAGE, &[("authorization", &authorization)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let listed = body["owned_spaces"]["spaces"].as_array().cloned();
    let listed = listed.unwrap_or_else(|| panic!("no owned spaces in {body}"));
    (listed.iter())
        .map(|space| {
            let field_of = |field: &str| match &space[field] {
                Value::String(given) => given.clone(),
                other => panic!("{field}: {other} in {space}"),
            };
            (field_of("space"), field_of("name"))
        })
        .collect()
}

/// alice created `acme`, where bob is an admin too and carol and dave members. Once her account is
/// erased, `acme` is still listed for bob, carol and dave, and each one's owned spaces are what
/// they were before her deletion, without `acme`.
#[test]
fn a_space_it_created_counts_against_no_accounts_owned_spaces() {
    let github = 3_304_020_011;
    let mut f = Fixture::new(github);
    let (phone, _) = alices_records(&mut f);
    let t = &f.team;
    let api = Api::of(&t.flow);
    let others = [&t.bob, &t.carol, &t.dave];
    let before: Vec<_> = others.iter().map(|person| owned_by(&api, person)).collect();
    let acme = t.space.to_string();
    for owned in &before {
        assert!(!owned.contains_key(&acme), "acme owned before: {owned:?}");
    }
    let devices = [t.alice.laptop.id, t.alice.uncertified.id, phone.id];
    let traces = Traces::of(&t.alice, github, &devices);
    let kept = Kept::of_space(&t.bob.session, t.space, &devices);

    deletes(&api, &t.alice);
    move_deletion_back(f.db(), t.alice.account, ERASED_AFTER);
    erased_within(f.db(), &traces, &kept);

    for (person, owned) in others.iter().zip(&before) {
        let listed = person.session.spaces();
        assert!(listed.contains_key(&acme), "{}: acme listed", person.handle);
        assert_eq!(
            &owned_by(&api, person),
            owned,
            "{}'s owned spaces",
            person.handle
        );
    }
    assert_still_there(&f, t.space);
}

/// Checks that `space`'s revisions are all still given to bob's session.
#[track_caller]
fn assert_still_there(f: &Fixture, space: SpaceId) {
    let given = f.team.bob.session.revisions(space);
    assert_eq!(given, f.revisions, "acme's revisions");
}
