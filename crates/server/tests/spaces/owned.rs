//! The limit on the spaces an account owns (`docs/dev/architecture.md`, "Cloud limits"). The
//! server setting `--owned-spaces-limit <n>`, or `SKALD_OWNED_SPACES_LIMIT`, gives the number of
//! spaces an account may own: the ones it created, its personal space included, which counts also
//! before the server creates it. Unset, there is no limit.
//!
//! A creation beyond the limit is refused with `403` and the problem
//! `https://skald.so/problems/owned-spaces-limit`, and creates nothing. The personal space is never
//! refused, and asking again for an own space that has no revision yet gives that same space.

use std::sync::Barrier;
use std::thread;

use skald_proto::{Role, SpaceId};

use crate::api::{Person, space_of};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::assert_taken;
use crate::common::sign_in::SignIn;
use crate::rosters::{first, member, next};

/// The server setting, as an argument and as an environment variable.
pub const LIMIT: &str = "--owned-spaces-limit";
pub const LIMIT_ENV: &str = "SKALD_OWNED_SPACES_LIMIT";

/// The problem of a creation beyond the limit.
pub const OVER_LIMIT: &str = "https://skald.so/problems/owned-spaces-limit";

/// A server whose accounts may own `limit` spaces, and whose codes poll every second.
fn limited(limit: &str) -> DeviceFlow {
    DeviceFlow::start_with(&[INTERVAL, "1", LIMIT, limit])
}

/// Checks that `response` refuses a creation beyond the limit; `what` names the case.
#[track_caller]
fn assert_over_limit(response: &Response, what: &str) {
    assert_eq!(response.status, 403, "{what}: {}", text(response));
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{what}: {}: {}",
        response.content_type,
        text(response)
    );
    assert_eq!(
        json(response)["type"],
        OVER_LIMIT,
        "{what}: {}",
        text(response)
    );
}

/// Whether `response` took a creation: `200` or `201`.
fn taken(response: &Response) -> bool {
    matches!(response.status, 200 | 201)
}

/// The names of the spaces `person`'s account gets, sorted.
fn names(person: &Person) -> Vec<String> {
    let mut names: Vec<String> = person.session.spaces().into_values().collect();
    names.sort();
    names
}

/// With the setting at 2, an account with its personal space creates one team space; a third is
/// refused, also once the team space has a revision, and also when asked again. The refused name
/// stays free: another account creates a space with it.
#[test]
fn with_the_limit_at_2_an_account_with_its_personal_space_creates_one_team_space() {
    let flow = limited("2");
    let alice = Person::new(&flow, 3_120_910_001, "alice");
    let (personal, personal_name) = alice.session.personal();

    let acme = alice.create_space("acme");
    assert_over_limit(&alice.session.create("globex"), "a third space");
    assert_over_limit(
        &alice.session.create("globex"),
        "the third space asked again",
    );
    let revision = alice.sign(&first(acme, "acme", &alice));
    assert_taken(&alice.session.store(acme, &revision));
    assert_over_limit(&alice.session.create("initech"), "after revision 1");

    let mut expected = vec!["acme".to_owned(), personal_name];
    expected.sort();
    assert_eq!(names(&alice), expected);
    assert!(alice.session.spaces().contains_key(&personal.to_string()));

    let bob = Person::new(&flow, 3_120_910_002, "bob");
    let globex = bob.create_space("globex");
    assert!(
        !alice.session.spaces().contains_key(&globex.to_string()),
        "alice gets bob's space"
    );
}

/// The personal space counts before the server creates it: an account without it yet creates one
/// team space, a second is refused, and the account still gets its personal space.
#[test]
fn account_without_its_personal_space_yet_creates_one_team_space_and_still_gets_it() {
    let flow = limited("2");
    let alice = Person::new(&flow, 3_120_910_011, "alice");

    alice.create_space("acme");
    assert_over_limit(&alice.session.create("globex"), "a second team space");
    let created = alice.session.personal_response();
    assert_eq!(created.status, 201, "{}", text(&created));
    let (_, personal_name) = alice.session.personal();
    assert_over_limit(&alice.session.create("initech"), "after the personal space");

    let mut expected = vec!["acme".to_owned(), personal_name];
    expected.sort();
    assert_eq!(names(&alice), expected);
}

/// The setting gives the number: at 1, an account creates no team space, with or without its
/// personal space, and still gets its personal space.
#[test]
fn with_the_limit_at_1_an_account_creates_no_team_space_and_still_gets_its_personal_space() {
    let flow = limited("1");
    let alice = Person::new(&flow, 3_120_910_021, "alice");

    assert_over_limit(&alice.session.create("acme"), "before the personal space");
    let (_, personal_name) = alice.session.personal();
    assert_over_limit(&alice.session.create("acme"), "after the personal space");
    assert_eq!(names(&alice), vec![personal_name]);
}

/// With the setting at 3, an account with its personal space creates two team spaces, and a third
/// is refused.
#[test]
fn with_the_limit_at_3_an_account_with_its_personal_space_creates_two_team_spaces() {
    let flow = limited("3");
    let alice = Person::new(&flow, 3_120_910_031, "alice");
    alice.session.personal();

    alice.create_space("acme");
    alice.create_space("globex");
    assert_over_limit(&alice.session.create("initech"), "a fourth space");
    assert_eq!(alice.session.spaces().len(), 3);
}

/// Sends `requests` from `person`'s session at once and gives their answers in order: the request
/// for the personal space if `None`, otherwise the creation of the space it names.
fn at_once(person: &Person, requests: &[Option<String>]) -> Vec<Response> {
    let ready = Barrier::new(requests.len());
    thread::scope(|scope| {
        let running: Vec<_> = requests
            .iter()
            .map(|request| {
                let (session, ready) = (person.session.clone(), &ready);
                scope.spawn(move || {
                    ready.wait();
                    match request {
                        Some(name) => session.create(name),
                        None => session.personal_response(),
                    }
                })
            })
            .collect();
        running
            .into_iter()
            .map(|thread| thread.join().expect("a request at once"))
            .collect()
    })
}

/// The creations of `count` spaces named for `round`.
fn creations(round: u64, count: usize) -> Vec<Option<String>> {
    (0..count)
        .map(|one| Some(format!("team-{round}-{one}")))
        .collect()
}

/// Fails unless one of `answers` took its creation and each other one refused it as beyond the
/// limit; then unless `person`'s account gets 2 spaces.
#[track_caller]
fn assert_one_taken(person: &Person, answers: &[Response], round: u64) {
    let statuses: Vec<String> = answers.iter().map(text).collect();
    let created = answers.iter().filter(|answer| taken(answer)).count();
    assert_eq!(created, 1, "round {round}: {statuses:?}");
    for refused in answers.iter().filter(|answer| !taken(answer)) {
        assert_over_limit(refused, &format!("round {round}"));
    }
    let spaces = person.session.spaces();
    assert_eq!(spaces.len(), 2, "round {round}: {spaces:?}");
}

/// Of creations at once from an account with its personal space, at the limit of 2, one is taken
/// and the others refused; the account owns 2 spaces.
#[test]
fn creations_at_once_never_pass_the_limit() {
    let flow = limited("2");
    for round in 0..3 {
        let person = Person::new(&flow, 3_120_910_041 + round, &format!("eve{round}"));
        person.session.personal();
        let answers = at_once(&person, &creations(round, 4));
        assert_one_taken(&person, &answers, round);
    }
}

/// Of creations at once from an account without its personal space, sent with the request for it,
/// at the limit of 2: the account gets its personal space, one creation is taken and the others
/// refused.
#[test]
fn creations_at_once_with_the_personal_space_request_never_pass_the_limit() {
    let flow = limited("2");
    for round in 0..3 {
        let person = Person::new(&flow, 3_120_910_051 + round, &format!("eve{round}"));
        let mut requests = vec![None];
        requests.extend(creations(round, 3));
        let mut answers = at_once(&person, &requests);
        let personal = answers.remove(0);
        assert_eq!(personal.status, 201, "round {round}: {}", text(&personal));
        assert_one_taken(&person, &answers, round);
        let spaces = person.session.spaces();
        assert!(spaces.contains_key(&space_of(&personal).to_string()));
    }
}

/// At the limit, the same creation again, as a retry after a lost answer, gives the same space
/// while it has no revision, with or without the personal space; another name is still refused.
#[test]
fn at_the_limit_a_retry_of_a_creation_whose_answer_was_lost_gives_the_same_space() {
    let flow = limited("2");
    let alice = Person::new(&flow, 3_120_910_061, "alice");
    alice.session.personal();
    let acme = alice.create_space("acme");

    let again = alice.session.create("acme");
    assert_eq!(again.status, 200, "{}", text(&again));
    assert_eq!(space_of(&again), acme, "another space");
    assert_over_limit(&alice.session.create("globex"), "after the retry");
    assert_eq!(alice.session.spaces().len(), 2);

    let bob = Person::new(&flow, 3_120_910_062, "bob");
    let initech = bob.create_space("initech");
    let again = bob.session.create("initech");
    assert_eq!(again.status, 200, "{}", text(&again));
    assert_eq!(space_of(&again), initech, "another space");
    bob.session.personal();
    let again = bob.session.create("initech");
    assert_eq!(again.status, 200, "{}", text(&again));
    assert_eq!(space_of(&again), initech, "another space");
    assert_eq!(bob.session.spaces().len(), 2);
}

/// An account owns only the spaces it created: a space it is an admin of, which another account
/// created, does not count for it, and still counts for its creator.
#[test]
fn spaces_an_account_did_not_create_do_not_count() {
    let flow = limited("2");
    let alice = Person::new(&flow, 3_120_910_071, "alice");
    let bob = Person::new(&flow, 3_120_910_072, "bob");
    alice.session.personal();
    let acme = alice.create_space("acme");
    let revision_1 = alice.sign(&first(acme, "acme", &alice));
    assert_taken(&alice.session.store(acme, &revision_1));
    let mut body = next(&revision_1, &alice.laptop);
    body.members.push(member(&bob, Role::Admin));
    assert_taken(&alice.session.store(acme, &alice.sign(&body)));
    assert!(bob.session.spaces().contains_key(&acme.to_string()));

    bob.session.personal();
    bob.create_space("globex");
    assert_over_limit(&bob.session.create("initech"), "bob's third own space");
    assert_eq!(
        bob.session.spaces().len(),
        3,
        "bob gets acme beside his own"
    );
    assert_over_limit(&alice.session.create("initech"), "alice's third own space");
}

/// Without the setting, an account owns any number of spaces. Started again on the same database
/// with the setting at 2, by its environment variable, the server keeps the accounts' spaces,
/// gives an account over the limit its personal space and the same space again while it has no
/// revision, and refuses its next creation.
#[test]
fn without_the_setting_there_is_no_limit_and_with_it_an_account_over_the_limit_creates_no_more() {
    let flow = DeviceFlow::start();
    let alice = Person::new(&flow, 3_120_910_081, "alice");
    let bob = Person::new(&flow, 3_120_910_082, "bob");
    alice.session.personal();
    let alice_spaces: Vec<SpaceId> = (1..=4)
        .map(|one| alice.create_space(&format!("alice-team-{one}")))
        .collect();
    for one in 1..=3 {
        bob.create_space(&format!("bob-team-{one}"));
    }
    assert_eq!(alice.session.spaces().len(), 5);
    assert_eq!(bob.session.spaces().len(), 3);

    let SignIn { db, server, .. } = flow.stand;
    drop(server);
    let flow = DeviceFlow::of(SignIn::on_with(db, &[INTERVAL, "1"], &[(LIMIT_ENV, "2")]));
    let (alice_session, bob_session) = (alice.session.at(&flow), bob.session.at(&flow));

    assert_eq!(alice_session.spaces().len(), 5);
    let again = alice_session.create("alice-team-1");
    assert_eq!(again.status, 200, "{}", text(&again));
    assert_eq!(space_of(&again), alice_spaces[0], "another space");
    assert_over_limit(
        &alice_session.create("alice-team-5"),
        "alice over the limit",
    );

    let personal = bob_session.personal_response();
    assert_eq!(personal.status, 201, "{}", text(&personal));
    assert_over_limit(&bob_session.create("bob-team-4"), "bob over the limit");
    assert_eq!(alice_session.spaces().len(), 5);
    assert_eq!(bob_session.spaces().len(), 4);
}
