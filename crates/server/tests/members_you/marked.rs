//! Which member the members' answer marks as the session's own account, and how the spec gives it.

use serde_json::Value;
use skald_proto::SpaceId;

use crate::api::Person;
use crate::common::device_flow::json;
use crate::common::devices::assert_taken;
use crate::listed::{MEMBERS, Web, members_response};
use crate::rosters::first;
use crate::sessions::Api;

/// The property that marks the session's own account.
const YOU: &str = "you";

/// The space dave creates beside `acme`, its only member.
const GLOBEX: &str = "globex";

/// The handles of the members of `space` that the request with `headers` gets marked as its own
/// account, in the answer's order; fails unless it gets them, or if a member's mark is neither
/// absent nor a boolean.
#[track_caller]
fn marked(api: &Api, space: SpaceId, headers: &[(&str, &str)]) -> Vec<String> {
    let response = members_response(api, space, headers);
    let body = json(&response);
    let (200, Value::Array(items)) = (response.status, &body["members"]) else {
        panic!("{}: not a list of members: {body}", response.status);
    };
    let mut handles = Vec::new();
    for item in items {
        match item.get(YOU) {
            None | Some(Value::Bool(false)) => {}
            Some(Value::Bool(true)) => handles.push(handle_of(item)),
            Some(other) => panic!("{YOU} {other}: absent or a boolean in {item}"),
        }
    }
    handles
}

/// The handle of the listed member `item`; the whole member without one.
fn handle_of(item: &Value) -> String {
    (item["handle"].as_str()).map_or_else(|| item.to_string(), str::to_owned)
}

/// The handle of `person`, as the only one marked.
fn only(person: &Person) -> Vec<String> {
    vec![person.handle.to_string()]
}

/// `acme` has alice and bob as admins and carol as a member. Each one's web session gets the
/// space's members with its own account marked and no other: an admin's and a member's alike.
#[test]
fn a_web_session_gets_its_own_account_marked_and_no_other() {
    let web = Web::new(3_302_600_001);
    let (team, api) = (&web.team, web.api());
    team.with_members();
    for (cookie, person) in [
        (&web.alice, &team.alice),
        (&web.bob, &team.bob),
        (&web.carol, &team.carol),
    ] {
        assert_eq!(
            marked(&api, team.space, &[("cookie", cookie)]),
            only(person),
            "{}'s web session",
            person.handle
        );
    }
}

/// The device sessions of alice, an admin of `acme`, and carol, a member, each get their own
/// account marked and no other; so does dave's, of `globex`, the space he alone is in.
#[test]
fn a_device_session_gets_its_own_account_marked_and_no_other() {
    let web = Web::new(3_302_600_101);
    let (team, api) = (&web.team, web.api());
    team.with_members();
    let dave = &team.dave;
    let globex = dave.create_space(GLOBEX);
    assert_taken(
        &dave
            .session
            .store(globex, &dave.sign(&first(globex, GLOBEX, dave))),
    );

    for (space, person) in [
        (team.space, &team.alice),
        (team.space, &team.carol),
        (globex, dave),
    ] {
        let bearer = format!("Bearer {}", person.session.token());
        assert_eq!(
            marked(&api, space, &[("authorization", &bearer)]),
            only(person),
            "{}'s device session",
            person.handle
        );
    }
}

/// `value`, with a `$ref` in `spec` followed until it is none.
fn resolved<'a>(spec: &'a Value, mut value: &'a Value) -> &'a Value {
    while let Some(Value::String(target)) = value.get("$ref") {
        let at = target.trim_start_matches('#');
        value = spec
            .pointer(at)
            .unwrap_or_else(|| panic!("a $ref that resolves: {target}"));
    }
    value
}

/// The spec's member of `GET /spaces/{space}/members` has the boolean property `you`.
#[test]
fn the_spec_gives_each_member_the_boolean_mark_of_the_sessions_account() {
    let spec = crate::common::spec::spec();
    let path = MEMBERS.replace('/', "~1");
    let pointer = format!("/paths/{path}/get/responses/200/content/application~1json/schema");
    let answer = resolved(
        &spec,
        spec.pointer(&pointer)
            .unwrap_or_else(|| panic!("no JSON answer 200 to GET {MEMBERS} in the spec")),
    );
    let member = resolved(&spec, &answer["properties"]["members"]["items"]);
    let you = resolved(&spec, &member["properties"][YOU]);
    assert_eq!(you["type"], "boolean", "the member's {YOU}: {member:#}");
}
