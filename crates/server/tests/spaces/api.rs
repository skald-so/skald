//! The accounts of the tests and their requests to the spaces API.

use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::OnceLock;

use serde_json::{Value, json};
use skald_proto::{AccountId, ChannelId, Name, SpaceId};

use crate::common::device_flow::{DeviceFlow, INTERVAL, json, text};
use crate::common::devices::{
    Device, Key, account_id, assert_taken, b64, devices_of, machine, unb64,
};
use crate::common::sign_in::is_uuid_v7;
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, Server, get_url, send};

/// Creates a space: `POST`; the account's spaces: `GET`.
pub const SPACES: &str = "/spaces";

/// The spec, read once.
fn the_spec() -> &'static Value {
    static SPEC: OnceLock<Value> = OnceLock::new();
    SPEC.get_or_init(spec)
}

/// The spec's path that `path` matches, a parameter matching any segment, such as
/// `/spaces/{space}/revisions` for `/spaces/<ID>/revisions`; `path` itself if none does.
fn template(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let paths = the_spec()["paths"].as_object().expect("paths in the spec");
    let matches = |candidate: &str| {
        let parts: Vec<&str> = candidate.split('/').collect();
        parts.len() == segments.len()
            && parts.iter().zip(&segments).all(|(part, segment)| {
                part == segment || (part.starts_with('{') && part.ends_with('}'))
            })
    };
    paths
        .keys()
        .filter(|candidate| matches(candidate))
        // A literal segment wins over a parameter.
        .min_by_key(|candidate| candidate.matches('{').count())
        .cloned()
        .unwrap_or_else(|| path.to_owned())
}

/// A session on the server, or none: plain data, so that threads share it.
#[derive(Clone)]
pub struct Session {
    base: String,
    token: Option<String>,
}

impl Session {
    /// Requests to `flow`'s server without a session.
    pub fn none(flow: &DeviceFlow) -> Self {
        Session {
            base: flow.stand.server.base().to_owned(),
            token: None,
        }
    }

    /// The same session, its requests to `flow`'s server, such as one started again on the same
    /// database.
    pub fn at(&self, flow: &DeviceFlow) -> Self {
        self.on(&flow.stand.server)
    }

    /// The same session, its requests to `server`, such as another instance on the same database.
    pub fn on(&self, server: &Server) -> Self {
        Session {
            base: server.base().to_owned(),
            token: self.token.clone(),
        }
    }

    /// `<method> <path>`, `method` `get` or `post`, with the JSON `body` if given; checked against
    /// the spec.
    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Response {
        self.call_stating(skald_proto::PROTOCOL_VERSION, method, path, body)
    }

    /// As [`Session::call`], stating the protocol version `version`.
    pub fn call_stating(
        &self,
        version: NonZeroU64,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Response {
        let url = format!("{}{path}", self.base);
        let version = version.to_string();
        let mut headers = vec![(skald_proto::VERSION_HEADER, version.as_str())];
        let bearer = self.token.as_ref().map(|token| format!("Bearer {token}"));
        if let Some(bearer) = &bearer {
            headers.push(("authorization", bearer));
        }
        let response = match (method, body) {
            ("get", None) => get_url(&url, &headers),
            (_, Some(body)) => {
                headers.push(("content-type", "application/json"));
                send(
                    method,
                    &url,
                    &headers,
                    &serde_json::to_vec(body).expect("JSON"),
                )
            }
            (_, None) => send(method, &url, &headers, &[]),
        };
        let violations = operation_violations(the_spec(), method, &template(path), &response);
        assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
        response
    }

    /// `POST /spaces` of `name`.
    pub fn create(&self, name: &str) -> Response {
        self.call("post", SPACES, Some(&json!({ "name": name })))
    }

    /// `GET /spaces`.
    pub fn spaces_response(&self) -> Response {
        self.call("get", SPACES, None)
    }

    /// The spaces this session's account gets: each name by its space's ID.
    pub fn spaces(&self) -> BTreeMap<String, String> {
        let response = self.spaces_response();
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        let listed = body["spaces"]
            .as_array()
            .unwrap_or_else(|| panic!("no spaces in {body}"));
        let mut spaces = BTreeMap::new();
        for space in listed {
            let id = space["space"].as_str().expect("a space ID").to_owned();
            let name = space["name"].as_str().expect("a space name").to_owned();
            assert!(
                spaces.insert(id.clone(), name).is_none(),
                "{id} twice: {body}"
            );
        }
        spaces
    }

    /// `POST /spaces/<space>/revisions` of `revision`.
    pub fn store(&self, space: SpaceId, revision: &[u8]) -> Response {
        let body = json!({ "revision": b64(revision) });
        self.call("post", &format!("/spaces/{space}/revisions"), Some(&body))
    }

    /// `GET /spaces/<space>/revisions`.
    pub fn revisions_response(&self, space: SpaceId) -> Response {
        self.call("get", &format!("/spaces/{space}/revisions"), None)
    }

    /// The revisions of `space` this session gets, in order.
    pub fn revisions(&self, space: SpaceId) -> Vec<Vec<u8>> {
        let response = self.revisions_response(space);
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        body["revisions"]
            .as_array()
            .unwrap_or_else(|| panic!("no revisions in {body}"))
            .iter()
            .map(|revision| unb64(revision.as_str().expect("a revision's bytes")))
            .collect()
    }

    /// `GET /spaces/<space>/devices`.
    pub fn devices_response(&self, space: SpaceId) -> Response {
        self.call("get", &format!("/spaces/{space}/devices"), None)
    }

    /// The devices of `space` this session gets: each device's certificate by its ID.
    pub fn devices(&self, space: SpaceId) -> BTreeMap<String, Option<Vec<u8>>> {
        devices_of(&self.devices_response(space))
    }

    /// `POST /spaces/<space>/channels`.
    pub fn channel_response(&self, space: SpaceId) -> Response {
        self.call("post", &format!("/spaces/{space}/channels"), None)
    }

    /// A new channel ID for `space`, which the server must give this session.
    pub fn channel(&self, space: SpaceId) -> ChannelId {
        let response = self.channel_response(space);
        assert_taken(&response);
        let body = json(&response);
        let id = body["channel"].as_str().expect("a channel ID");
        assert!(is_uuid_v7(id), "channel ID {id:?} is not a UUIDv7");
        id.parse().expect("a channel ID")
    }
}

/// An account signed in on a machine, with its first device certified by its master key and a
/// second device it registered without a certificate.
pub struct Person {
    pub session: Session,
    pub account: AccountId,
    pub handle: Name,
    pub master: Key,
    pub laptop: Device,
    /// The certificate of `laptop`, as the machine sent it.
    pub cert: Vec<u8>,
    pub uncertified: Device,
}

impl Person {
    /// Signs in the GitHub user `github` on `flow`'s server and sets up its devices; `handle` is
    /// its handle in rosters.
    pub fn new(flow: &DeviceFlow, github: u64, handle: &str) -> Self {
        let user = flow.stand.user(github, &format!("{handle}-spaces-standin"));
        let signed_in = machine(flow, &user);
        let master = Key::generate();
        let laptop = Device::new("Laptop");
        let cert = laptop.cert(&signed_in.account, &master);
        signed_in.register_device(&laptop);
        assert_taken(&signed_in.give_master(&master, &cert));
        let uncertified = Device::new("Desktop");
        signed_in.register_device(&uncertified);
        Person {
            session: Session {
                base: flow.stand.server.base().to_owned(),
                token: Some(signed_in.token.clone()),
            },
            account: account_id(&signed_in.account),
            handle: handle.parse().expect("a valid handle"),
            master,
            laptop,
            cert,
            uncertified,
        }
    }

    /// Creates the space `name`, which the server must take; gives its ID.
    pub fn create_space(&self, name: &str) -> SpaceId {
        let response = self.session.create(name);
        assert_taken(&response);
        space_of(&response)
    }
}

/// The space ID in `response`, a `POST /spaces` answer.
pub fn space_of(response: &Response) -> SpaceId {
    let body = json(response);
    let id = body["space"]
        .as_str()
        .unwrap_or_else(|| panic!("no space in {body}"));
    assert!(is_uuid_v7(id), "space ID {id:?} is not a UUIDv7");
    id.parse().expect("a space ID")
}

/// Checks that `response` refuses with a `4xx` and a problem details object; `what` names the case.
pub fn assert_refused(response: &Response, what: &str) {
    assert!(
        (400..500).contains(&response.status),
        "{what}: refused, not {}",
        text(response)
    );
    let problem = response
        .content_type
        .starts_with("application/problem+json");
    assert!(
        problem,
        "{what}: {}: {}",
        response.content_type,
        text(response)
    );
}

/// The accounts of most tests on a server of their own: alice, who created the space `acme`, and
/// bob, carol and dave. Each test gives its own range of GitHub user IDs.
pub struct Team {
    pub flow: DeviceFlow,
    pub alice: Person,
    pub bob: Person,
    pub carol: Person,
    pub dave: Person,
    pub space: SpaceId,
}

/// The space's name.
pub const ACME: &str = "acme";

impl Team {
    /// The team, with `acme` created and no revision stored; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        Self::with(github, &[])
    }

    /// As [`Team::new`], on a server with the extra arguments `args`.
    pub fn with(github: u64, args: &[&str]) -> Self {
        let mut all = vec![INTERVAL, "1"];
        all.extend_from_slice(args);
        let flow = DeviceFlow::start_with(&all);
        let alice = Person::new(&flow, github, "alice");
        let bob = Person::new(&flow, github + 1, "bob");
        let carol = Person::new(&flow, github + 2, "carol");
        let dave = Person::new(&flow, github + 3, "dave");
        let space = alice.create_space(ACME);
        Team {
            flow,
            alice,
            bob,
            carol,
            dave,
            space,
        }
    }
}
