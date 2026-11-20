//! What the tests share: the space `acme` with alice's `service` device listed as `ci/deploy-bot`,
//! the grants the tests sign, and the requests that create, list, revoke and use tokens.

use serde_json::{Value, json};
use skald_proto::{
    Cbor, ChannelId, DeviceId, DeviceKind, Grant, GrantBody, NonEmpty, PROTOCOL_VERSION, Role,
    ServiceAgent, SpaceId, Unknown, VERSION_HEADER,
};

use crate::api::{Person, template, the_spec};
use crate::common::device_flow::{json, text};
use crate::common::devices::{Device, assert_taken, b64, unb64};
use crate::common::spec::operation_violations;
use crate::common::{Response, TestDb, delete_url, get_url, send};
use crate::fixture::Fixture;
use crate::https::Https;
use crate::rosters::{first, name, service_agent};
use crate::wire::now;

/// A token reads its own record and grant: `GET`, with the token as its bearer token.
pub const OWN: &str = "/service-token";
/// The label of a grant's signature.
pub const GRANT_LABEL: &str = "skald grant";
/// The permissions a token may have.
pub const SEND: &str = "send";
pub const RECEIVE: &str = "receive";
pub const QUEUES: &str = "queues";
/// A day, in seconds.
pub const DAY: u64 = 86_400;
/// The `Origin` of a page of another site.
pub const ELSEWHERE: &str = "https://elsewhere.example";

/// The space's tokens: `POST` creates one, `GET` lists them.
pub fn tokens_path(space: SpaceId) -> String {
    format!("/spaces/{space}/service-tokens")
}

/// The grant of `service` for `channels`, signed by `signer` until `expires`, encoded.
pub fn grant(service: DeviceId, channels: &[ChannelId], signer: &Device, expires: u64) -> Vec<u8> {
    let body = GrantBody {
        service,
        channels: NonEmpty::from_vec(channels.to_vec()).expect("a channel"),
        signer: signer.id,
        expires,
        unknown: Unknown::default(),
    };
    let signature = signer.signing.key.sign(GRANT_LABEL, &[&body.to_cbor()]);
    Grant { body, signature }.to_cbor()
}

/// The body of a token's creation.
pub fn creation(device: DeviceId, grant: &[u8], permissions: &[&str]) -> Value {
    json!({ "device": device.to_string(), "grant": b64(grant), "permissions": permissions })
}

/// `<method> <path>` to the server at `base`, `method` `get`, `post` or `delete`, stating the
/// protocol version, with `headers` and, for `post`, the JSON `body`; checked against the spec.
pub fn call(
    base: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Response {
    let version = PROTOCOL_VERSION.to_string();
    let mut all = vec![(VERSION_HEADER, version.as_str())];
    all.extend_from_slice(headers);
    let url = format!("{base}{path}");
    let response = match method {
        "get" => get_url(&url, &all),
        "delete" => delete_url(&url, &all),
        _ => {
            all.push(("content-type", "application/json"));
            let bytes = serde_json::to_vec(body.expect("a body")).expect("JSON");
            send(method, &url, &all, &bytes)
        }
    };
    let violations = operation_violations(the_spec(), method, &template(path), &response);
    assert_eq!(
        violations,
        Vec::<String>::new(),
        "{method} {path}: {}",
        text(&response)
    );
    response
}

/// The `Authorization` header of the bearer token `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// A token the server gave.
#[derive(Debug, Clone)]
pub struct Created {
    /// The value, the token itself.
    pub token: String,
    pub id: String,
    /// The whole answer.
    pub body: Value,
}

/// The fixture's space with alice's `service` device `bot`, certified by her laptop, listed in
/// revision 4 as `ci/deploy-bot`.
pub struct Stage {
    pub f: Fixture,
    pub bot: Device,
}

impl Stage {
    /// The stage; GitHub user IDs from `github` on.
    pub fn new(github: u64) -> Self {
        let mut f = Fixture::new(github);
        let alice = &f.team.alice;
        let bot = alice.certified_by(f.db(), &alice.laptop, DeviceKind::Service, "Deploy bot");
        f.revise(|body| body.service_agents.push(service_agent(&bot)));
        Stage { f, bot }
    }

    pub fn space(&self) -> SpaceId {
        self.f.team.space
    }

    /// alice's laptop stores the next revision, without the bot's entry.
    pub fn drop_entry(&mut self) {
        let bot = self.bot.id;
        (self.f).revise(|body| body.service_agents.retain(|agent| agent.device != bot));
    }

    pub fn base(&self) -> &str {
        self.f.team.flow.stand.server.base()
    }

    /// The server's public address, the `Origin` of its pages.
    pub fn public(&self) -> &str {
        &self.f.team.flow.stand.public
    }

    pub fn db(&self) -> &TestDb {
        self.f.db()
    }

    /// The grant of the bot for `general`, by alice's laptop, for a day.
    pub fn bot_grant(&self) -> Vec<u8> {
        let alice = &self.f.team.alice;
        grant(self.bot.id, &[self.f.general], &alice.laptop, now() + DAY)
    }

    /// The proof of `device` on a new challenge of `person`'s session.
    pub fn proof(&self, person: &Person, device: &Device) -> String {
        let flow = &self.f.team.flow;
        Https::to(&flow.stand.server, flow, person, device).proof()
    }

    /// `POST /spaces/<acme>/service-tokens` of `body` from `person`'s session, with `proof` if
    /// given.
    pub fn create_response(&self, person: &Person, proof: Option<&str>, body: &Value) -> Response {
        let bearer = bearer(person.session.token());
        let mut headers = vec![("authorization", bearer.as_str())];
        if let Some(proof) = proof {
            headers.push((crate::https::PROOF, proof));
        }
        call(
            self.base(),
            "post",
            &tokens_path(self.space()),
            &headers,
            Some(body),
        )
    }

    /// Creates a token of `device` with `grant` and `permissions` from `person`'s session, proved
    /// by `person`'s laptop, which the server must give.
    #[track_caller]
    pub fn create_as(
        &self,
        person: &Person,
        device: DeviceId,
        grant: &[u8],
        permissions: &[&str],
    ) -> Created {
        let proof = self.proof(person, &person.laptop);
        let body = creation(device, grant, permissions);
        let response = self.create_response(person, Some(&proof), &body);
        assert_eq!(response.status, 201, "a token: {}", text(&response));
        let body = json(&response);
        let token = body["token"]
            .as_str()
            .expect("the token's value")
            .to_owned();
        assert!(!token.is_empty(), "an empty token: {body}");
        let id = body["id"].as_str().expect("the token's ID").to_owned();
        Created { token, id, body }
    }

    /// Creates a token of the bot with `grant`, sending and receiving, from alice's session.
    #[track_caller]
    pub fn create_with(&self, grant: &[u8]) -> Created {
        self.create_as(&self.f.team.alice, self.bot.id, grant, &[SEND, RECEIVE])
    }

    /// Creates a token of the bot with [`Stage::bot_grant`], sending and receiving, from alice's
    /// session.
    #[track_caller]
    pub fn create(&self) -> Created {
        self.create_with(&self.bot_grant())
    }

    /// `GET /spaces/<acme>/service-tokens` with `headers`.
    pub fn list_response(&self, headers: &[(&str, &str)]) -> Response {
        call(
            self.base(),
            "get",
            &tokens_path(self.space()),
            headers,
            None,
        )
    }

    /// The tokens `person`'s device session lists.
    #[track_caller]
    pub fn list(&self, person: &Person) -> Vec<Value> {
        let bearer = bearer(person.session.token());
        tokens_of(&self.list_response(&[("authorization", &bearer)]))
    }

    /// The IDs of the tokens `person`'s device session lists, sorted.
    #[track_caller]
    pub fn listed(&self, person: &Person) -> Vec<String> {
        let mut ids: Vec<String> = self
            .list(person)
            .iter()
            .map(|token| token["id"].as_str().expect("a token's ID").to_owned())
            .collect();
        ids.sort();
        ids
    }

    /// `DELETE /spaces/<acme>/service-tokens/<id>` with `headers`.
    pub fn revoke_response(&self, id: &str, headers: &[(&str, &str)]) -> Response {
        let path = format!("{}/{id}", tokens_path(self.space()));
        call(self.base(), "delete", &path, headers, None)
    }

    /// `DELETE /spaces/<acme>/service-tokens/<id>` from `person`'s device session.
    pub fn revoke_as(&self, person: &Person, id: &str) -> Response {
        let bearer = bearer(person.session.token());
        self.revoke_response(id, &[("authorization", &bearer)])
    }

    /// `GET /service-token` with `token`.
    pub fn own(&self, token: &str) -> Response {
        let bearer = bearer(token);
        call(self.base(), "get", OWN, &[("authorization", &bearer)], None)
    }

    /// Whether the server takes `token`: `200` to `GET /service-token`, or `401` with a problem.
    #[track_caller]
    pub fn works(&self, token: &str) -> bool {
        let response = self.own(token);
        match response.status {
            200 => true,
            401 => {
                let problem = response
                    .content_type
                    .starts_with("application/problem+json");
                assert!(problem, "a problem: {}", text(&response));
                false
            }
            _ => panic!("taken or refused: {}", text(&response)),
        }
    }

    /// The `Cookie` header of a new web session of `person`, who signed in as the GitHub user
    /// `github`: the spaces tests' people sign in as `<handle>-spaces-standin`.
    pub fn cookie(&self, person: &Person, github: u64) -> String {
        let flow = &self.f.team.flow;
        let login = format!("{}-spaces-standin", person.handle);
        flow.web_cookie(&flow.stand.user(github, &login))
    }

    /// Stores the next revision of `acme`, with `account` an admin in it.
    pub fn make_admin(&mut self, account: skald_proto::AccountId) {
        self.f.revise(|body| {
            let mut members = body.members.clone().into_vec();
            for member in members.iter_mut().filter(|m| m.account == account) {
                member.role = Role::Admin;
            }
            body.members = NonEmpty::from_vec(members).expect("members");
        });
    }

    /// Stores the next revision of `acme`, with `device` listed as the service agent `ci/<agent>`.
    pub fn list_agent(&mut self, device: &Device, agent: &str) {
        let entry = ServiceAgent {
            device: device.id,
            owner: name("ci"),
            agent: name(agent),
            unknown: Unknown::default(),
        };
        self.f.revise(|body| body.service_agents.push(entry));
    }

    /// A channel of another space, `globex`, which eve creates and is the admin of.
    pub fn other_channel(&self) -> (SpaceId, ChannelId) {
        let eve = &self.f.eve;
        let globex = eve.create_space("globex");
        assert_taken(
            &eve.session
                .store(globex, &eve.sign(&first(globex, "globex", eve))),
        );
        (globex, eve.session.channel(globex))
    }
}

/// The tokens in `response`, a `GET /spaces/<space>/service-tokens` answer.
#[track_caller]
pub fn tokens_of(response: &Response) -> Vec<Value> {
    assert_eq!(response.status, 200, "{}", text(response));
    let body = json(response);
    body["tokens"]
        .as_array()
        .unwrap_or_else(|| panic!("no tokens in {body}"))
        .clone()
}

/// The token `id` among `tokens`.
#[track_caller]
pub fn find<'a>(tokens: &'a [Value], id: &str) -> &'a Value {
    tokens
        .iter()
        .find(|token| token["id"] == id)
        .unwrap_or_else(|| panic!("{id} not in {tokens:?}"))
}

/// The strings of `value`, an array of them, sorted.
#[track_caller]
pub fn sorted_strings(value: &Value) -> Vec<String> {
    let mut strings: Vec<String> = value
        .as_array()
        .unwrap_or_else(|| panic!("an array: {value}"))
        .iter()
        .map(|item| item.as_str().expect("a string").to_owned())
        .collect();
    strings.sort();
    strings
}

/// The seconds since the Unix epoch of `at`, an RFC 3339 date-time, as PostgreSQL reads it.
#[track_caller]
pub fn seconds(db: &TestDb, at: &Value) -> i64 {
    let at = at
        .as_str()
        .unwrap_or_else(|| panic!("a date-time: {at}"))
        .to_owned();
    db.query(async |conn| {
        let (seconds,): (i64,) =
            sqlx::query_as("SELECT floor(extract(epoch FROM $1::text::timestamptz))::bigint")
                .bind(&at)
                .fetch_one(conn)
                .await?;
        Ok(seconds)
    })
    .unwrap_or_else(|err| panic!("{at:?} is no date-time: {err}"))
}

/// `grant`, an encoded grant, with the last byte of its signature changed.
pub fn broken(grant: &[u8]) -> Vec<u8> {
    let mut bytes = grant.to_vec();
    *bytes.last_mut().expect("a grant") ^= 1;
    bytes
}

/// The grant's bytes in `body`, a token's own record.
#[track_caller]
pub fn grant_of(body: &Value) -> Vec<u8> {
    unb64(
        body["grant"]
            .as_str()
            .unwrap_or_else(|| panic!("no grant in {body}")),
    )
}
