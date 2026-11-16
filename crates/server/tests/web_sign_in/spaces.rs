//! The account's spaces list marks its personal space, so that the web app never asks for it: only
//! the account's own personal space, to its device and web sessions alike, and a list asked for
//! before the personal space exists creates none.

use std::collections::BTreeMap;

use serde_json::Value;
use skald_github_stand_in::User;
use skald_proto::{Role, SpaceId};

use crate::api::{Person, Session, space_of};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::assert_taken;
use crate::rosters::{first, member, next};

/// The session's account's personal space: `POST`.
const PERSONAL: &str = "/account/personal-space";

/// Whether each space in `body`, a `200` answer to `GET /spaces`, is marked personal, by its ID: a
/// space without the mark is not.
fn marks(body: &Value) -> BTreeMap<String, bool> {
    body["spaces"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of spaces: {body}"))
        .iter()
        .map(|space| {
            let id = space["space"].as_str().expect("a space ID").to_owned();
            let personal = match space.get("personal") {
                None | Some(Value::Bool(false)) => false,
                Some(Value::Bool(true)) => true,
                Some(other) => panic!("personal {other} in {body}"),
            };
            (id, personal)
        })
        .collect()
}

/// The marks of the spaces `session`'s account gets.
fn marks_of(session: &Session) -> BTreeMap<String, bool> {
    let response = session.spaces_response();
    assert_eq!(response.status, 200, "{}", text(&response));
    marks(&json(&response))
}

/// The marks of the spaces the account of the web session `cookie` gets; checked against the spec.
fn web_marks(flow: &DeviceFlow, cookie: &str) -> BTreeMap<String, bool> {
    let response = flow.request("get", "/spaces", &[("cookie", cookie)], None, true);
    assert_eq!(response.status, 200, "{}", text(&response));
    marks(&json(&response))
}

/// The personal space of `person`'s account, which the request creates or gives.
fn personal(person: &Person) -> (SpaceId, String) {
    let response = person.session.call("post", PERSONAL, None);
    assert_taken(&response);
    let name = json(&response)["name"].as_str().expect("a name").to_owned();
    (space_of(&response), name)
}

/// The GitHub user `Person::new` signs in as `handle` with the ID `id`.
fn user_of(id: u64, handle: &str) -> User {
    User::new(id, &format!("{handle}-spaces-standin"))
}

/// Ada has her personal space, a team space she created and bob's personal space, whose revision
/// adds her: her list marks only her own personal space, to her device session and her web session
/// alike; bob's marks only his.
#[test]
fn spaces_list_marks_the_accounts_personal_space_and_no_other() {
    let flow = DeviceFlow::start();
    let ada = Person::new(&flow, 3_220_400_001, "ada");
    let bob = Person::new(&flow, 3_220_400_002, "bob");
    let team = ada.create_space("acme");
    let (own, _) = personal(&ada);
    let (bobs, bobs_name) = personal(&bob);
    let revision_1 = bob.sign(&first(bobs, &bobs_name, &bob));
    assert_taken(&bob.session.store(bobs, &revision_1));
    let mut body = next(&revision_1, &bob.laptop);
    body.members.push(member(&ada, Role::Member));
    assert_taken(&bob.session.store(bobs, &bob.sign(&body)));

    let expected = BTreeMap::from([
        (own.to_string(), true),
        (team.to_string(), false),
        (bobs.to_string(), false),
    ]);
    assert_eq!(marks_of(&ada.session), expected, "ada's device session");
    let cookie = flow.web_cookie(&user_of(3_220_400_001, "ada"));
    assert_eq!(web_marks(&flow, &cookie), expected, "ada's web session");
    assert_eq!(
        marks_of(&bob.session),
        BTreeMap::from([(bobs.to_string(), true)]),
        "bob's"
    );
}

/// A list asked for before the account's personal space exists creates none: the account's first
/// request for it creates it afterwards, and the list then marks it.
#[test]
fn spaces_list_before_the_personal_space_creates_none() {
    let flow = DeviceFlow::start();
    let cy = Person::new(&flow, 3_220_400_011, "cy");
    let cookie = flow.web_cookie(&user_of(3_220_400_011, "cy"));
    for _ in 0..2 {
        assert_eq!(marks_of(&cy.session), BTreeMap::new(), "device session");
        assert_eq!(web_marks(&flow, &cookie), BTreeMap::new(), "web session");
    }

    let created = cy.session.call("post", PERSONAL, None);
    assert_eq!(created.status, 201, "created now: {}", text(&created));
    let space = space_of(&created).to_string();
    assert_eq!(
        marks_of(&cy.session),
        BTreeMap::from([(space.clone(), true)])
    );
    assert_eq!(web_marks(&flow, &cookie), BTreeMap::from([(space, true)]));
}
