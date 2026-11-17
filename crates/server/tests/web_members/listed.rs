//! A space's members as `GET /spaces/{space}/members` gives them, and the people of the tests with
//! a web session each.

use std::collections::BTreeMap;

use serde_json::Value;
use skald_proto::{Member, NonEmpty, Role, RosterBody, SpaceId};

use crate::api::{Person, Team};
use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::sessions::{Api, epoch};

/// A space's members as the web shows them: `GET`.
pub const MEMBERS: &str = "/spaces/{space}/members";

/// The path of `space`'s members.
pub fn members_path(space: SpaceId) -> String {
    format!("/spaces/{space}/members")
}

/// A member as the list gives it, its date-time in seconds since the Unix epoch.
#[derive(Clone, Debug)]
pub struct Shown {
    pub role: Role,
    /// The GitHub login of its account's GitHub method.
    pub login: Option<String>,
    pub linked_devices: u64,
    pub joined_at: i64,
    /// The member as listed.
    pub item: Value,
}

/// `GET /spaces/<space>/members` with `headers`, checked against the spec.
pub fn members_response(api: &Api, space: SpaceId, headers: &[(&str, &str)]) -> Response {
    api.call("get", &members_path(space), MEMBERS, headers)
}

/// The members of `space` that the web session `cookie` gets, by handle; fails unless it gets
/// them, or if two have one handle.
#[track_caller]
pub fn members(api: &Api, space: SpaceId, cookie: &str) -> BTreeMap<String, Shown> {
    let response = members_response(api, space, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let items = body["members"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of members: {body}"));
    let mut by_handle = BTreeMap::new();
    for item in items {
        let field = |name: &str| {
            item.get(name)
                .unwrap_or_else(|| panic!("no {name} in {item}"))
        };
        let handle = field("handle").as_str().expect("a handle").to_owned();
        let role = match field("role").as_str() {
            Some("admin") => Role::Admin,
            Some("member") => Role::Member,
            other => panic!("a role, admin or member: {other:?} in {item}"),
        };
        let login = match item.get("github_login") {
            None => None,
            Some(Value::String(login)) => Some(login.clone()),
            Some(other) => panic!("github_login {other}: absent or a login in {item}"),
        };
        let shown = Shown {
            role,
            login,
            linked_devices: field("linked_devices")
                .as_u64()
                .unwrap_or_else(|| panic!("a count of linked devices in {item}")),
            joined_at: epoch(api.db, field("joined_at").as_str().expect("a date-time")),
            item: item.clone(),
        };
        assert!(
            by_handle.insert(handle.clone(), shown).is_none(),
            "{handle} twice: {body}"
        );
    }
    by_handle
}

/// The role of each member of `space` that the web session `cookie` gets, by handle.
#[track_caller]
pub fn roles(api: &Api, space: SpaceId, cookie: &str) -> BTreeMap<String, Role> {
    members(api, space, cookie)
        .into_iter()
        .map(|(handle, member)| (handle, member.role))
        .collect()
}

/// `people`, each with its role, by handle.
pub fn expected(people: &[(&Person, Role)]) -> BTreeMap<String, Role> {
    people
        .iter()
        .map(|(person, role)| (person.handle.to_string(), *role))
        .collect()
}

/// `body` with the members `change` leaves of its members.
pub fn members_changed(mut body: RosterBody, change: impl FnOnce(&mut Vec<Member>)) -> RosterBody {
    let mut list = body.members.into_vec();
    change(&mut list);
    body.members = NonEmpty::from_vec(list).expect("a member left");
    body
}

/// Sets `person`'s role in `members` to `role`.
pub fn set_role(members: &mut [Member], person: &Person, role: Role) {
    members
        .iter_mut()
        .find(|entry| entry.account == person.account)
        .unwrap_or_else(|| panic!("{} a member", person.handle))
        .role = role;
}

/// The GitHub login of a person of the spaces tests whose handle is `handle`.
pub fn login_of(handle: &str) -> String {
    format!("{handle}-spaces-standin")
}

/// The team of the spaces tests, `acme` created and no revision stored, with the `Cookie` header of
/// a web session of each of its people.
pub struct Web {
    pub team: Team,
    pub alice: String,
    pub bob: String,
    pub carol: String,
    pub dave: String,
}

impl Web {
    /// The team with GitHub user IDs from `github` on, as [`Team::new`] gives them.
    pub fn new(github: u64) -> Self {
        let team = Team::new(github);
        let cookie = |n: u64, person: &Person| {
            let user = team
                .flow
                .stand
                .user(github + n, &login_of(person.handle.as_str()));
            team.flow.web_cookie(&user)
        };
        let alice = cookie(0, &team.alice);
        let bob = cookie(1, &team.bob);
        let carol = cookie(2, &team.carol);
        let dave = cookie(3, &team.dave);
        Web {
            team,
            alice,
            bob,
            carol,
            dave,
        }
    }

    pub fn api(&self) -> Api<'_> {
        Api::of(&self.team.flow)
    }
}
