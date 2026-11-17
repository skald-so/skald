//! The account's devices as `GET /account/devices` gives them, and the people whose devices the
//! tests make.

use std::collections::BTreeMap;

use serde_json::Value;
use skald_proto::DeviceId;

use crate::api::Person;
use crate::common::Response;
use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::sessions::{Api, epoch};

/// The account's devices as the web shows them: `GET`.
pub const ACCOUNT_DEVICES: &str = "/account/devices";

/// A device's state by the server's checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Linked,
    Waiting,
    Revoked,
}

/// A device as the list gives it, its date-time in seconds since the Unix epoch.
#[derive(Clone, Debug)]
pub struct Shown {
    pub id: DeviceId,
    pub name: String,
    pub kind: String,
    pub registered_at: i64,
    pub state: State,
    /// The device as listed.
    pub item: Value,
}

/// A server whose codes poll every second.
pub fn flow() -> DeviceFlow {
    DeviceFlow::start_with(&[INTERVAL, "1"])
}

/// A person with an account on `flow`'s server, as the spaces tests make one: a master key, the
/// device `Laptop` certified by it and the device `Desktop` registered without a certificate; and
/// the `Cookie` header of a web session of theirs.
pub fn person(flow: &DeviceFlow, github: u64, handle: &str) -> (Person, String) {
    let person = Person::new(flow, github, handle);
    let user = flow.stand.user(github, &format!("{handle}-spaces-standin"));
    (person, flow.web_cookie(&user))
}

/// `GET /account/devices` with `headers`, checked against the spec.
pub fn list_response(api: &Api, headers: &[(&str, &str)]) -> Response {
    api.call("get", ACCOUNT_DEVICES, ACCOUNT_DEVICES, headers)
}

/// The devices the web session `cookie` gets, by name; fails unless it gets them, or if two have
/// one name.
#[track_caller]
pub fn shown(api: &Api, cookie: &str) -> BTreeMap<String, Shown> {
    let response = list_response(api, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    let mut by_name = BTreeMap::new();
    for device in listed(api, &json(&response)) {
        let name = device.name.clone();
        assert!(
            by_name.insert(name.clone(), device).is_none(),
            "{name} twice"
        );
    }
    by_name
}

/// The devices in `body`, a `200` answer to `GET /account/devices`.
pub fn listed(api: &Api, body: &Value) -> Vec<Shown> {
    let items = body["devices"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of devices: {body}"));
    items
        .iter()
        .map(|item| {
            let string = |name: &str| {
                item[name]
                    .as_str()
                    .unwrap_or_else(|| panic!("no {name} in {item}"))
                    .to_owned()
            };
            let state = match string("state").as_str() {
                "linked" => State::Linked,
                "waiting" => State::Waiting,
                "revoked" => State::Revoked,
                other => panic!("a state, linked, waiting or revoked: {other} in {item}"),
            };
            Shown {
                id: string("device")
                    .parse()
                    .unwrap_or_else(|err| panic!("a device ID in {item}: {err}")),
                name: string("name"),
                kind: string("kind"),
                registered_at: epoch(api.db, &string("registered_at")),
                state,
                item: item.clone(),
            }
        })
        .collect()
}

/// The state of each device `cookie`'s web session gets, by name.
#[track_caller]
pub fn states(api: &Api, cookie: &str) -> BTreeMap<String, State> {
    shown(api, cookie)
        .into_iter()
        .map(|(name, device)| (name, device.state))
        .collect()
}
