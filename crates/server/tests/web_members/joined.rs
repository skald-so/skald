//! When each member joined: when the server stored the revision that added it.

use std::thread;
use std::time::Duration;

use skald_proto::Role;

use crate::api::{ACME, Person};
use crate::listed::{Web, members, members_changed, set_role};
use crate::rosters::{first, member, next};
use crate::sessions::now;

/// How far the database's clock and the test's may be apart, in seconds.
const SLACK: i64 = 5;
/// How long the test waits between two revisions.
const APART: Duration = Duration::from_secs(2);

/// Revision 1 of `acme` adds alice; revision 2, stored later, adds bob and carol; revision 3,
/// stored later still, makes bob an admin and adds dave. Each member's `joined_at` is when the
/// server stored the revision that added it: bob's and carol's are one time, after alice's, and
/// bob's stays that of revision 2, before dave's.
#[test]
fn each_member_joined_when_the_server_stored_the_revision_that_added_it() {
    let web = Web::new(3_225_200_001);
    let team = &web.team;
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);

    let mut stored_between = Vec::new();
    let mut store = |revision: &[u8]| {
        let before = now();
        team.stored_by(alice, revision);
        stored_between.push((before, now()));
        thread::sleep(APART);
    };
    let one = alice.sign(&first(team.space, ACME, alice));
    store(&one);
    let mut body = next(&one, &alice.laptop);
    body.members.push(member(bob, Role::Member));
    body.members.push(member(carol, Role::Member));
    let two = alice.sign(&body);
    store(&two);
    let body = members_changed(next(&two, &alice.laptop), |list| {
        set_role(list, bob, Role::Admin);
        list.push(member(dave, Role::Member));
    });
    store(&alice.sign(&body));

    let shown = members(&web.api(), team.space, &web.dave);
    let joined = |person: &Person| {
        shown
            .get(person.handle.as_str())
            .unwrap_or_else(|| panic!("{} not listed: {shown:#?}", person.handle))
            .joined_at
    };
    for (person, revision) in [(alice, 0), (bob, 1), (carol, 1), (dave, 2)] {
        let (before, after) = stored_between[revision];
        assert!(
            (before - SLACK..=after + SLACK).contains(&joined(person)),
            "{} joined at {}, revision {} stored within {before}..={after}",
            person.handle,
            joined(person),
            revision + 1
        );
    }
    assert_eq!(
        joined(bob),
        joined(carol),
        "one revision added bob and carol"
    );
    assert!(joined(alice) < joined(bob), "{shown:#?}");
    assert!(joined(bob) < joined(dave), "{shown:#?}");
    assert_eq!(shown[bob.handle.as_str()].role, Role::Admin, "{shown:#?}");
}
