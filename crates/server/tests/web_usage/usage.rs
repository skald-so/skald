//! The account's and a space's usage as `GET /account/usage` and `GET /spaces/{space}/usage` give
//! them, and the people of the tests with a web session each.

use std::collections::BTreeMap;

use serde_json::Value;
use skald_proto::SpaceId;

use crate::api::{Person, space_of};
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::fixture::Fixture;
use crate::sessions::Api;

/// The account's owned spaces and devices: `GET`.
pub const ACCOUNT_USAGE: &str = "/account/usage";
/// A space's limits and messages this month: `GET`.
pub const SPACE_USAGE: &str = "/spaces/{space}/usage";
/// The session's account's personal space: `POST`.
const PERSONAL: &str = "/account/personal-space";

/// The path of `space`'s usage.
pub fn space_usage_path(space: SpaceId) -> String {
    format!("/spaces/{space}/usage")
}

/// A count and its limit, `None` while no limit applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counted {
    pub count: u64,
    pub limit: Option<u64>,
}

/// `count` of `limit`.
pub fn of(count: u64, limit: u64) -> Counted {
    Counted {
        count,
        limit: Some(limit),
    }
}

/// `count`, without a limit.
pub fn unlimited(count: u64) -> Counted {
    Counted { count, limit: None }
}

/// The account's usage.
#[derive(Clone, Debug)]
pub struct AccountUsage {
    pub owned: Counted,
    /// The names of the owned spaces listed, by space ID.
    pub owned_spaces: BTreeMap<String, String>,
    pub devices: Counted,
}

/// A space's usage.
#[derive(Clone, Debug)]
pub struct SpaceUsage {
    pub messages: Counted,
    /// The largest payload, in bytes.
    pub message_size: Option<u64>,
    /// How long an undelivered message waits, in seconds.
    pub retention: Option<u64>,
}

/// The `limit` of the object `field` of `body`: `None` if it is absent or `null`.
fn limit_in(body: &Value, field: &str) -> Option<u64> {
    let object = body
        .get(field)
        .unwrap_or_else(|| panic!("no {field} in {body}"));
    match object.get("limit") {
        None | Some(Value::Null) => None,
        Some(limit) => Some(
            limit
                .as_u64()
                .unwrap_or_else(|| panic!("{field}'s limit, a number: {body}")),
        ),
    }
}

/// The count and limit of the object `field` of `body`.
fn counted_in(body: &Value, field: &str) -> Counted {
    let count = body[field]["count"]
        .as_u64()
        .unwrap_or_else(|| panic!("{field}'s count, a number: {body}"));
    Counted {
        count,
        limit: limit_in(body, field),
    }
}

/// `GET /account/usage` with `headers`, checked against the spec.
pub fn account_usage_response(api: &Api, headers: &[(&str, &str)]) -> Response {
    api.call("get", ACCOUNT_USAGE, ACCOUNT_USAGE, headers)
}

/// The usage of the account of the web session `cookie`; fails unless it gets it.
#[track_caller]
pub fn account_usage(api: &Api, cookie: &str) -> AccountUsage {
    let response = account_usage_response(api, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let listed = body["owned_spaces"]["spaces"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of owned spaces: {body}"));
    let mut owned_spaces = BTreeMap::new();
    for item in listed {
        let field = |name: &str| {
            item[name]
                .as_str()
                .unwrap_or_else(|| panic!("no {name} in {item}"))
                .to_owned()
        };
        let space = field("space");
        assert!(
            owned_spaces.insert(space.clone(), field("name")).is_none(),
            "{space} twice: {body}"
        );
    }
    AccountUsage {
        owned: counted_in(&body, "owned_spaces"),
        owned_spaces,
        devices: counted_in(&body, "devices"),
    }
}

/// `GET /spaces/<space>/usage` with `headers`, checked against the spec.
pub fn space_usage_response(api: &Api, space: SpaceId, headers: &[(&str, &str)]) -> Response {
    api.call("get", &space_usage_path(space), SPACE_USAGE, headers)
}

/// The usage of `space` that the web session `cookie` gets; fails unless it gets it.
#[track_caller]
pub fn space_usage(api: &Api, space: SpaceId, cookie: &str) -> SpaceUsage {
    let response = space_usage_response(api, space, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    SpaceUsage {
        messages: counted_in(&body, "monthly_messages"),
        message_size: limit_in(&body, "message_size"),
        retention: limit_in(&body, "mailbox_retention"),
    }
}

/// The names of `spaces` by space ID, each a space and its name.
pub fn names(spaces: &[(SpaceId, &str)]) -> BTreeMap<String, String> {
    spaces
        .iter()
        .map(|(space, name)| (space.to_string(), (*name).to_owned()))
        .collect()
}

/// `POST /account/personal-space` from `person`'s session: the personal space, its name, and
/// whether this request created it.
#[track_caller]
pub fn personal_space(person: &Person) -> (SpaceId, String, bool) {
    let response = person.session.call("post", PERSONAL, None);
    assert!(
        matches!(response.status, 200 | 201),
        "the personal space: {}",
        text(&response)
    );
    let name = json(&response)["name"]
        .as_str()
        .unwrap_or_else(|| panic!("no name in {}", text(&response)))
        .to_owned();
    (space_of(&response), name, response.status == 201)
}

/// The fixture of the cloud limits' tests, with the `Cookie` header of a web session of each of
/// its people.
pub struct Web {
    pub fixture: Fixture,
    pub alice: String,
    pub bob: String,
    pub carol: String,
    pub dave: String,
    pub eve: String,
}

impl Web {
    /// The fixture on a server with the extra arguments `args`, GitHub user IDs from `github` on,
    /// as [`Fixture::with`] gives them.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let fixture = Fixture::with(github, args);
        let flow: &DeviceFlow = &fixture.team.flow;
        let mut cookies = ["alice", "bob", "carol", "dave", "eve"]
            .into_iter()
            .zip(github..)
            .map(|(handle, id)| {
                flow.web_cookie(&flow.stand.user(id, &format!("{handle}-spaces-standin")))
            });
        let mut next = || cookies.next().expect("a cookie");
        let (alice, bob, carol, dave, eve) = (next(), next(), next(), next(), next());
        Web {
            fixture,
            alice,
            bob,
            carol,
            dave,
            eve,
        }
    }

    pub fn api(&self) -> Api<'_> {
        Api::of(&self.fixture.team.flow)
    }
}
