//! The spaces an account owns, as the owned spaces limit counts them: the ones it created, its
//! personal space included also before it is created.

use std::collections::BTreeMap;

use crate::api::{ACME, assert_refused};
use crate::common::device_flow::text;
use crate::usage::{Web, account_usage, names, of, personal_space};

/// The setting the tests give.
const LIMIT: &str = "--owned-spaces-limit";

/// With the setting at 2: alice, who created `acme` and has no personal space yet, owns 2 of 2,
/// `acme` listed; bob, a member of `acme` who created nothing, owns 1 of 2, none listed. Asking
/// created no personal space: alice's first request for it creates it, and she still owns 2 of 2,
/// `acme` and her personal space listed; her next creation is refused. Bob creates `globex` and
/// then gets his personal space: 2 of 2, both listed. Each owns only their own.
#[test]
fn owned_spaces_count_the_personal_space_also_before_it_exists() {
    let web = Web::with(3_226_200_001, &[LIMIT, "2"]);
    let (team, api) = (&web.fixture.team, web.api());
    let (alice, bob) = (&team.alice, &team.bob);

    let usage = account_usage(&api, &web.alice);
    assert_eq!(usage.owned, of(2, 2), "alice, before her personal space");
    assert_eq!(usage.owned_spaces, names(&[(team.space, ACME)]), "alice");
    let usage = account_usage(&api, &web.bob);
    assert_eq!(usage.owned, of(1, 2), "bob, before his personal space");
    assert_eq!(usage.owned_spaces, BTreeMap::new(), "bob");

    let (personal, name, created) = personal_space(alice);
    assert!(
        created,
        "asking for the usage created alice's personal space"
    );
    let usage = account_usage(&api, &web.alice);
    assert_eq!(usage.owned, of(2, 2), "alice, with her personal space");
    let both = names(&[(team.space, ACME), (personal, &name)]);
    assert_eq!(usage.owned_spaces, both, "alice, with her personal space");
    let refused = alice.session.create("initech");
    assert_refused(&refused, "alice's third space");
    assert_eq!(
        account_usage(&api, &web.alice).owned,
        of(2, 2),
        "{}",
        text(&refused)
    );

    let globex = bob.create_space("globex");
    let (personal, name, created) = personal_space(bob);
    assert!(created, "asking for the usage created bob's personal space");
    let usage = account_usage(&api, &web.bob);
    assert_eq!(
        usage.owned,
        of(2, 2),
        "bob, with globex and his personal space"
    );
    let both = names(&[(globex, "globex"), (personal, &name)]);
    assert_eq!(
        usage.owned_spaces, both,
        "bob, with globex and his personal space"
    );
}
