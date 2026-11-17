//! Members and roles as the space's latest stored revision gives them.

use skald_proto::Role;

use crate::api::{ACME, assert_refused};
use crate::common::device_flow::text;
use crate::listed::{
    Web, expected, login_of, members, members_changed, members_response, roles, set_role,
};
use crate::rosters::{member, next};

/// Revision 2 of `acme` lists alice and bob as admins and carol as a member: each of them gets
/// that, each member with the GitHub login of its account. Revision 3 makes bob a member and adds
/// dave as an admin; revision 4, alice's removal of carol, leaves her out: each time the members
/// get the latest revision's members and roles, and carol, once removed, gets nothing.
#[test]
fn members_and_roles_follow_the_latest_stored_revision_also_after_a_removal() {
    let web = Web::new(3_225_100_001);
    let api = web.api();
    let team = &web.team;
    let (alice, bob, carol, dave) = (&team.alice, &team.bob, &team.carol, &team.dave);
    let [_, second] = team.with_members();

    let shown = expected(&[
        (alice, Role::Admin),
        (bob, Role::Admin),
        (carol, Role::Member),
    ]);
    for (cookie, who) in [
        (&web.alice, "alice"),
        (&web.bob, "bob"),
        (&web.carol, "carol"),
    ] {
        assert_eq!(roles(&api, team.space, cookie), shown, "{who}, revision 2");
    }
    for (handle, listed) in members(&api, team.space, &web.carol) {
        assert_eq!(listed.login, Some(login_of(&handle)), "{listed:?}");
    }

    let body = members_changed(next(&second, &alice.laptop), |list| {
        set_role(list, bob, Role::Member);
        list.push(member(dave, Role::Admin));
    });
    let third = alice.sign(&body);
    team.stored_by(alice, &third);
    let shown = expected(&[
        (alice, Role::Admin),
        (bob, Role::Member),
        (carol, Role::Member),
        (dave, Role::Admin),
    ]);
    for (cookie, who) in [(&web.bob, "bob"), (&web.dave, "dave")] {
        assert_eq!(roles(&api, team.space, cookie), shown, "{who}, revision 3");
    }

    let body = members_changed(next(&third, &alice.laptop), |list| {
        list.retain(|entry| entry.account != carol.account);
    });
    team.stored_by(alice, &alice.sign(&body));
    let shown = expected(&[
        (alice, Role::Admin),
        (bob, Role::Member),
        (dave, Role::Admin),
    ]);
    for (cookie, who) in [(&web.alice, "alice"), (&web.dave, "dave")] {
        assert_eq!(roles(&api, team.space, cookie), shown, "{who}, revision 4");
    }
    let removed = members_response(&api, team.space, &[("cookie", &web.carol)]);
    assert_refused(&removed, "carol, removed");
    let body = text(&removed);
    for handle in [
        alice.handle.as_str(),
        bob.handle.as_str(),
        dave.handle.as_str(),
    ] {
        assert!(!body.contains(handle), "carol, removed: {body}");
    }
    assert!(!body.contains(ACME), "carol, removed: {body}");
}
