//! Creating a space: the server issues its ID, refuses an invalid or taken name, and gives the same
//! space again to its creator while it has no revision.

use crate::api::{ACME, Person, SPACES, Session, Team, assert_refused, space_of};
use crate::common::device_flow::{DeviceFlow, text};
use crate::common::devices::assert_taken;
use crate::rosters::first;

#[test]
fn a_space_is_created_and_the_same_request_gives_it_again_while_it_has_no_revision() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_120_800_001, "alice");

    let created = alice.session.create(ACME);
    assert_taken(&created);
    let space = space_of(&created);
    let spaces = alice.session.spaces();
    assert_eq!(spaces.get(&space.to_string()), Some(&ACME.to_owned()));

    // A retry after a lost answer.
    let again = alice.session.create(ACME);
    assert_taken(&again);
    assert_eq!(space_of(&again), space);
    let spaces = alice.session.spaces();
    let named: Vec<_> = spaces.values().filter(|name| *name == ACME).collect();
    assert_eq!(named.len(), 1, "one space named {ACME}: {spaces:?}");
}

#[test]
fn an_invalid_name_creates_no_space() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_120_800_011, "alice");
    let too_long = "a".repeat(64);
    let invalid = [
        "",
        "Acme",
        "acme corp",
        "acme_corp",
        "acme.corp",
        "acme/corp",
        "-acme",
        "acme-",
        "acmé",
        " acme",
        "acme\n",
        too_long.as_str(),
    ];
    for name in invalid {
        assert_refused(&alice.session.create(name), &format!("name {name:?}"));
    }
    let spaces = alice.session.spaces();
    for name in invalid {
        assert!(
            spaces.values().all(|listed| listed != name),
            "{name:?} in {spaces:?}"
        );
    }
}

#[test]
fn another_spaces_name_is_refused() {
    let team = Team::new(3_120_800_021);

    // Before the space has a revision: the name is taken all the same.
    assert_refused(&team.bob.session.create(ACME), "before revision 1");
    let revision = team.alice.sign(&first(team.space, ACME, &team.alice));
    team.stored_by(&team.alice, &revision);
    assert_refused(&team.bob.session.create(ACME), "after revision 1");

    assert!(
        !team.bob.session.spaces().values().any(|name| name == ACME),
        "bob gets no space named {ACME}"
    );
    assert_eq!(team.alice.session.revisions(team.space), vec![revision]);
}

#[test]
fn creating_a_space_needs_a_session() {
    let flow = DeviceFlow::start();
    let response = Session::none(&flow).create(ACME);
    assert_eq!(response.status, 401, "{}", text(&response));
    let listed = Session::none(&flow).call("get", SPACES, None);
    assert_eq!(listed.status, 401, "{}", text(&listed));
}
