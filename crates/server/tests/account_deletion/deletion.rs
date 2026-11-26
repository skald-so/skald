//! What the tests share: the requests that preview and delete the session's account and the checks
//! of their answers, the spaces an account alone is in, how a shared space shows a deleted member,
//! and what the database still holds of a space.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;
use skald_proto::{ChannelId, NonEmpty, Role, RosterBody, SpaceId};
use tungstenite::protocol::frame::coding::CloseCode;

use crate::api::{ACME, Person, Session, Team, space_of};
use crate::common::device_flow::{json, text};
use crate::common::devices::assert_taken;
use crate::common::{Closed, Response, TestDb, hex};
use crate::rosters::{channel, first, member, next};
use crate::sessions::Api;

/// Deletes the session's account: `DELETE`.
pub const ACCOUNT: &str = "/account";
/// What a deletion of the session's account deletes and what blocks it: `GET`.
pub const DELETION: &str = "/account/deletion";
/// The session's account's personal space: `POST`.
pub const PERSONAL: &str = "/account/personal-space";
/// The problem of a deletion a space blocks.
pub const BLOCKED: &str = "https://skald.so/problems/deletion-blocked";
/// The problem of a deletion whose session's sign-in is older than the setting.
pub const TOO_OLD: &str = "https://skald.so/problems/sign-in-too-old";
/// The reason the connections of a deleted account's sessions close with.
pub const ACCOUNT_DELETED: &str = "account-deleted";
/// How soon those connections close, on every instance.
pub const CLOSE: Duration = Duration::from_secs(1);
/// How far back a test moves a sign-in to make it older than the setting's 300 seconds.
pub const OLDER: &str = "10 minutes";
/// The page where a person approves a device code.
pub const VERIFICATION: &str = "/device";

/// How a deleted account's connection closes.
pub fn account_deleted() -> Option<Closed> {
    Some(Closed::Frame(CloseCode::Policy, ACCOUNT_DELETED.to_owned()))
}

/// The `Authorization` value of the bearer token `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// `DELETE /account` with `headers`.
pub fn delete_with(api: &Api, headers: &[(&str, &str)]) -> Response {
    api.call("delete", ACCOUNT, ACCOUNT, headers)
}

/// The device session `token` deletes its account.
pub fn delete_by_token(api: &Api, token: &str) -> Response {
    let authorization = bearer(token);
    delete_with(api, &[("authorization", &authorization)])
}

/// The web session `cookie` deletes its account, from a page of the server.
pub fn delete_by_cookie(api: &Api, cookie: &str) -> Response {
    delete_with(api, &[("cookie", cookie), ("origin", api.public)])
}

/// `person`'s device session deletes its account, which the server must delete.
#[track_caller]
pub fn deletes(api: &Api, person: &Person) {
    assert_deleted(&delete_by_token(api, person.session.token()));
}

/// Checks that `response` says the account is deleted: `204` without a body.
#[track_caller]
pub fn assert_deleted(response: &Response) {
    assert_eq!(response.status, 204, "deleted: {}", text(response));
    assert!(response.body.is_empty(), "{}", text(response));
}

/// Checks that `response` refuses: a `4xx` with a problem details object; `what` names the case.
#[track_caller]
pub fn assert_refused(response: &Response, what: &str) {
    let problem = response
        .content_type
        .starts_with("application/problem+json");
    assert!(
        (400..500).contains(&response.status) && problem,
        "{what}: refused with a problem, not {} {}: {}",
        response.status,
        response.content_type,
        text(response)
    );
}

/// The problem of a revoked device.
pub const DEVICE_REVOKED: &str = "https://skald.so/problems/device-revoked";

/// Checks that `response` refuses a device as revoked: `403` with `device-revoked`.
#[track_caller]
pub fn assert_revoked(response: &Response, what: &str) {
    assert_eq!(problem_of(response, what), DEVICE_REVOKED, "{what}");
    assert_eq!(response.status, 403, "{what}");
}

/// The problem type of `response`, a refusal.
#[track_caller]
pub fn problem_of(response: &Response, what: &str) -> String {
    assert_refused(response, what);
    let body = json(response);
    (body["type"].as_str())
        .unwrap_or_else(|| panic!("{what}: no problem type in {body}"))
        .to_owned()
}

/// Checks that `response` refuses a deletion that `blocking` block: `409` with the problem
/// `deletion-blocked` naming them.
#[track_caller]
pub fn assert_blocked(response: &Response, blocking: &[SpaceId]) {
    assert_eq!(
        problem_of(response, "blocked"),
        BLOCKED,
        "{}",
        text(response)
    );
    assert_eq!(response.status, 409, "{}", text(response));
    let body = json(response);
    assert_eq!(
        spaces_in(&body["spaces"], "spaces"),
        ids(blocking),
        "{body}"
    );
}

/// The IDs of `spaces`, as the answers give them.
pub fn ids(spaces: &[SpaceId]) -> BTreeSet<String> {
    spaces.iter().map(ToString::to_string).collect()
}

/// The space IDs of `list`, an array of objects that each give a space's ID as `space`.
fn spaces_in(list: &Value, what: &str) -> BTreeSet<String> {
    let items = (list.as_array()).unwrap_or_else(|| panic!("{what}: an array, not {list}"));
    let mut found = BTreeSet::new();
    for item in items {
        let id = item["space"].as_str();
        let id = id.unwrap_or_else(|| panic!("{what}: a space's ID in {item}"));
        assert!(found.insert(id.to_owned()), "{what}: {id} twice in {list}");
    }
    found
}

/// What a deletion of an account deletes and what blocks it, as the server says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub deletes: BTreeSet<String>,
    pub blocks: BTreeSet<String>,
}

/// `GET /account/deletion` with `headers`.
#[track_caller]
pub fn preview_with(api: &Api, headers: &[(&str, &str)]) -> Preview {
    let response = api.call("get", DELETION, DELETION, headers);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    Preview {
        deletes: spaces_in(&body["deletes"], "deletes"),
        blocks: spaces_in(&body["blocks"], "blocks"),
    }
}

/// What a deletion of `person`'s account deletes and what blocks it, as its device session asks.
#[track_caller]
pub fn preview(api: &Api, person: &Person) -> Preview {
    let authorization = bearer(person.session.token());
    preview_with(api, &[("authorization", &authorization)])
}

/// Checks that `person`'s device session still works, as before any deletion.
#[track_caller]
pub fn assert_works(api: &Api, person: &Person, what: &str) {
    assert!(api.device_works(person.session.token()), "{what}");
}

/// `person`'s personal space, with its revision 1 stored: `person` its only member.
#[track_caller]
pub fn personal_space(person: &Person) -> SpaceId {
    let response = person.session.call("post", PERSONAL, None);
    assert_taken(&response);
    let body = json(&response);
    let name = (body["name"].as_str()).unwrap_or_else(|| panic!("no name in {body}"));
    let space = space_of(&response);
    let revision = person.sign(&first(space, name, person));
    assert_taken(&person.session.store(space, &revision));
    space
}

/// A team space `name` that `person` creates, alone in it, with a channel of only `person`; gives
/// the space and the channel.
#[track_caller]
pub fn space_alone(person: &Person, name: &str) -> (SpaceId, ChannelId) {
    let space = person.create_space(name);
    let one = person.sign(&first(space, name, person));
    assert_taken(&person.session.store(space, &one));
    let id = person.session.channel(space);
    let mut body = next(&one, &person.laptop);
    body.channels.push(channel(id, &[person]));
    assert_taken(&person.session.store(space, &person.sign(&body)));
    (space, id)
}

/// The rows of `db` that hold `id`, a space's or a channel's, as its text or as the hex of its
/// bytes, such as in a revision's.
pub fn rows_holding(db: &TestDb, id: &str, bytes: &[u8; 16]) -> Vec<String> {
    let hexed = hex(bytes);
    let mut rows = db.rows();
    rows.retain(|row| row.contains(id) || row.contains(&hexed));
    rows
}

/// Checks that some row of `db` holds the space `space`.
#[track_caller]
pub fn assert_holds_space(db: &TestDb, space: SpaceId, what: &str) {
    let rows = rows_holding(db, &space.to_string(), space.as_bytes());
    assert!(!rows.is_empty(), "{what}: no row holds {space}");
}

/// Checks that no row of `db` holds the space `space` or any of `channels`, its channels.
#[track_caller]
pub fn assert_space_gone(db: &TestDb, space: SpaceId, channels: &[ChannelId], what: &str) {
    let rows = rows_holding(db, &space.to_string(), space.as_bytes());
    assert_eq!(
        rows,
        Vec::<String>::new(),
        "{what}: rows still hold {space}"
    );
    for id in channels {
        let rows = rows_holding(db, &id.to_string(), id.as_bytes());
        assert_eq!(rows, Vec::<String>::new(), "{what}: rows still hold {id}");
    }
}

/// The members of `space` that `session` gets: each member's handle and whether it is marked
/// deleted, in the answer's order.
#[track_caller]
pub fn members(session: &Session, space: SpaceId) -> Vec<(String, bool)> {
    let response = session.call("get", &format!("/spaces/{space}/members"), None);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let listed = (body["members"].as_array()).unwrap_or_else(|| panic!("no members in {body}"));
    listed
        .iter()
        .map(|member| {
            let handle = member["handle"].as_str().expect("a handle").to_owned();
            let deleted = match member.get("deleted") {
                None | Some(Value::Bool(false)) => false,
                Some(Value::Bool(true)) => true,
                Some(other) => panic!("deleted {other} in {member}"),
            };
            (handle, deleted)
        })
        .collect()
}

/// The accounts `response`, a `GET /spaces/<space>/devices` answer, marks deleted.
#[track_caller]
pub fn deleted_accounts(response: &Response) -> BTreeSet<String> {
    assert_eq!(response.status, 200, "{}", text(response));
    let body = json(response);
    let Some(listed) = body.get("deleted") else {
        return BTreeSet::new();
    };
    let listed = (listed.as_array()).unwrap_or_else(|| panic!("deleted, an array: {body}"));
    let mut accounts = BTreeSet::new();
    for account in listed {
        let id = account
            .as_str()
            .unwrap_or_else(|| panic!("an account ID: {body}"));
        assert!(accounts.insert(id.to_owned()), "{id} twice: {body}");
    }
    accounts
}

/// The count `sql` gives in `db`, `$1` bound to `id`, an ID as text.
pub fn count(db: &TestDb, sql: &'static str, id: &str) -> i64 {
    let (count,): (i64,) = db
        .query(async |conn| sqlx::query_as(sql).bind(id).fetch_one(conn).await)
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
    count
}

/// Gives `person` the role `role` among `body`'s members.
pub fn with_role(body: &mut RosterBody, person: &Person, role: Role) {
    let mut members = body.members.clone().into_vec();
    let found = members.iter_mut().find(|m| m.account == person.account);
    found.expect("among the members").role = role;
    body.members = NonEmpty::from_vec(members).expect("a member");
}

/// Stores revision 1 of `acme` from alice's session, and revision 2, which adds `person` as a
/// member, both signed by alice's laptop; gives them.
pub fn acme_with_member(team: &Team, person: &Person) -> [Vec<u8>; 2] {
    let alice = &team.alice;
    let one = alice.sign(&first(team.space, ACME, alice));
    team.stored_by(alice, &one);
    let mut body = next(&one, &alice.laptop);
    body.members.push(member(person, Role::Member));
    let two = alice.sign(&body);
    team.stored_by(alice, &two);
    [one, two]
}
