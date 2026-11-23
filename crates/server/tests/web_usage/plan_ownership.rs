//! Space creation and account usage agree on an account's Free slots.

use std::sync::Barrier;
use std::thread;

use skald_proto::SpaceId;

use crate::api::{ACME, Person, Session, Team, assert_refused, space_of};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json as body_of, text};
use crate::common::devices::assert_taken;
use crate::plans::{CLOUD, entitle_team};
use crate::usage::{ACCOUNT_USAGE, AccountUsage, account_usage_of, names, of, personal_space};

const OWNED_LIMIT: &str = "https://skald.so/problems/owned-spaces-limit";

#[track_caller]
fn owned(session: &Session) -> AccountUsage {
    account_usage_of(&session.call("get", ACCOUNT_USAGE, None))
}

#[track_caller]
fn assert_owned(usage: &AccountUsage, count: u64, expected_spaces: &[(SpaceId, &str)]) {
    assert_eq!(usage.owned, of(count, 2), "{usage:?}");
    assert_eq!(usage.owned_spaces, names(expected_spaces), "{usage:?}");
}

#[track_caller]
fn assert_owned_limit(response: &Response) {
    assert_refused(response, "the account's Free slots are full");
    assert_eq!(response.status, 403, "{}", text(response));
    assert_eq!(body_of(response)["type"], OWNED_LIMIT, "{}", text(response));
}

#[test]
fn free_ownership_counts_personal_and_excludes_paid_and_joined_spaces() {
    let team = Team::with(3_226_920_001, CLOUD);
    let (alice, bob) = (&team.alice, &team.bob);
    let db = &team.flow.stand.db;
    entitle_team(db, team.space);
    let paid = (team.space, ACME);

    assert_owned(&owned(&alice.session), 1, &[paid]);
    assert_owned(&owned(&bob.session), 1, &[]);
    team.with_members();
    assert!(bob.session.spaces().contains_key(&team.space.to_string()));
    assert_owned(&owned(&bob.session), 1, &[]);
    let bobs_free = bob.create_space("globex");
    assert_owned(&owned(&bob.session), 2, &[(bobs_free, "globex")]);

    let free = (alice.create_space("initech"), "initech");
    let before = owned(&alice.session);
    assert_owned(&before, 2, &[paid, free]);
    assert_owned_limit(&alice.session.create("umbrella"));
    assert_eq!(
        owned(&alice.session),
        before,
        "refused creation changed usage"
    );
    assert_eq!(alice.session.spaces(), names(&[paid, free]));

    let (personal, personal_name, created) = personal_space(alice);
    assert!(
        created,
        "usage and ordinary creation left the personal space uncreated"
    );
    let personal = (personal, personal_name.as_str());
    assert_owned(&owned(&alice.session), 2, &[paid, free, personal]);

    entitle_team(db, personal.0);
    assert_owned(&owned(&alice.session), 1, &[paid, free, personal]);
    let response = alice.session.create("umbrella");
    assert_eq!(response.status, 201, "{}", text(&response));
    let another_free = (space_of(&response), "umbrella");
    assert_owned(
        &owned(&alice.session),
        2,
        &[paid, free, personal, another_free],
    );
}

#[test]
fn two_instances_cannot_create_two_spaces_using_one_free_slot() {
    let flow = DeviceFlow::start_with(&[INTERVAL, "1", "--space-plans"]);
    let other = flow.stand.instance(CLOUD, &[]);

    for round in 0..5 {
        let person = Person::new(&flow, 3_226_920_011 + round, &format!("owner{round}"));
        let there = person.session.on(&other);
        let first_name = format!("acme-{round}");
        let second_name = format!("globex-{round}");
        let start = Barrier::new(2);
        let (first, second) = thread::scope(|scope| {
            let first = scope.spawn(|| {
                start.wait();
                person.session.create(&first_name)
            });
            let second = scope.spawn(|| {
                start.wait();
                there.create(&second_name)
            });
            (
                first.join().expect("first instance"),
                second.join().expect("second instance"),
            )
        });

        let (winner, loser, name) = match (first.status, second.status) {
            (201, 403) => (&first, &second, first_name.as_str()),
            (403, 201) => (&second, &first, second_name.as_str()),
            _ => panic!(
                "round {round}: first {}, second {}",
                text(&first),
                text(&second)
            ),
        };
        assert_taken(winner);
        assert_owned_limit(loser);
        let expected = [(space_of(winner), name)];
        let usage = owned(&person.session);
        assert_owned(&usage, 2, &expected);
        assert_eq!(owned(&there), usage);
        assert_eq!(person.session.spaces(), names(&expected));
        assert_eq!(there.spaces(), names(&expected));
    }
}
