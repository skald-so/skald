//! A space before its revision 1, a member removed and added again, and a member's device session.

use std::thread;
use std::time::Duration;

use serde_json::Value;
use skald_proto::Role;

use crate::common::device_flow::{json, text};
use crate::listed::{Web, members, members_changed, members_path, members_response};
use crate::rosters::{member, next};

/// How long the test waits before the revision that adds bob again, in seconds.
const APART: u64 = 3;

/// Before revision 1, alice, who created `acme`, gets no members. Revision 2 adds bob, revision 3
/// removes him and revision 4, stored `APART` seconds later, adds him again: he joined then.
#[test]
fn a_member_added_again_joined_when_the_revision_that_added_him_again_was_stored() {
    let web = Web::new(3_225_700_001);
    let (team, api) = (&web.team, web.api());
    let (alice, bob) = (&team.alice, &team.bob);
    let empty = members_response(&api, team.space, &[("cookie", &web.alice)]);
    assert_eq!(empty.status, 200, "{}", text(&empty));
    assert_eq!(json(&empty)["members"], Value::Array(vec![]));

    let [_, second] = team.with_members();
    let third = alice.sign(&members_changed(next(&second, &alice.laptop), |list| {
        list.retain(|entry| entry.account != bob.account);
    }));
    team.stored_by(alice, &third);
    thread::sleep(Duration::from_secs(APART));
    let mut body = next(&third, &alice.laptop);
    body.members.push(member(bob, Role::Member));
    team.stored_by(alice, &alice.sign(&body));

    let shown = members(&api, team.space, &web.bob);
    let joined = |handle: &str| shown[handle].joined_at;
    let (alice, bob, carol) = (
        alice.handle.as_str(),
        bob.handle.as_str(),
        team.carol.handle.as_str(),
    );
    assert_eq!(shown[bob].role, Role::Member, "{shown:#?}");
    assert!(joined(carol) - joined(alice) < 2, "{shown:#?}");
    assert!(
        joined(bob) - joined(carol) >= 2,
        "bob joined when revision 4 was stored: {shown:#?}"
    );
}

/// Carol's device session gets the members her web session gets.
#[test]
fn a_members_device_session_gets_the_members_as_a_web_session_does() {
    let web = Web::new(3_225_800_001);
    let team = &web.team;
    team.with_members();

    let response = team
        .carol
        .session
        .call("get", &members_path(team.space), None);
    assert_eq!(response.status, 200, "{}", text(&response));
    let by_web = members_response(&web.api(), team.space, &[("cookie", &web.carol)]);
    assert_eq!(json(&response), json(&by_web));
    assert_eq!(json(&response)["members"].as_array().map(Vec::len), Some(3));
}
