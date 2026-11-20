//! What the tests upload and read: `presence` values, written byte by byte as the server sees them,
//! a device's proved requests for a channel's presence, and the server's answers.

use std::time::{Duration, Instant};

use skald_proto::{ChannelId, DeviceId, SpaceId, VERSION_HEADER};

use crate::api::{Person, template, the_spec};
use crate::common::cbor::{byte_item, head, uint};
use crate::common::device_flow::{DeviceFlow, json, text};
use crate::common::devices::{Device, random, unb64};
use crate::common::spec::operation_violations;
use crate::common::{Response, Server, TestDb, get_url, hex, send};
use crate::https::{AUTH_REQUIRED, CBOR, Https, PROOF, problem};
use crate::scene::Scene;

/// The problem of a `ct` over [`LONGEST_CT`] bytes.
pub const TOO_LARGE: &str = "too-large";
/// The longest `ct` the server takes: 16 blocks of 4096 bytes and the tag.
pub const LONGEST_CT: usize = 16 * 4096 + 16;
/// The length of a `ct` of one padded block and its tag, as a device seals a short value.
pub const ONE_BLOCK: usize = 4096 + 16;
/// How long a test waits for the server to delete a value.
const GONE_WITHIN: Duration = Duration::from_secs(10);

/// The path of `channel`'s presence in `space`.
pub fn presence_path(space: SpaceId, channel: ChannelId) -> String {
    format!("/spaces/{space}/channels/{channel}/presence")
}

/// `n` random bytes.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(n + 32);
    while bytes.len() < n {
        bytes.extend_from_slice(&random());
    }
    bytes.truncate(n);
    bytes
}

/// A `presence` value as the server sees one: it cannot open `ct`.
#[derive(Debug, Clone)]
pub struct Value {
    pub device: DeviceId,
    pub channel: ChannelId,
    pub epoch: u64,
    pub nonce: Vec<u8>,
    pub ct: Vec<u8>,
    /// Fields of later versions: their keys, above 5, and encoded values.
    pub later: Vec<(u64, Vec<u8>)>,
}

impl Value {
    /// A value of `device` in `channel` under `epoch`, with a random nonce and a random `ct` of one
    /// block.
    pub fn new(device: DeviceId, channel: ChannelId, epoch: u64) -> Self {
        Value {
            device,
            channel,
            epoch,
            nonce: random_bytes(12),
            ct: random_bytes(ONE_BLOCK),
            later: Vec::new(),
        }
    }

    /// The value with a `ct` of `len` random bytes.
    pub fn with_ct(mut self, len: usize) -> Self {
        self.ct = random_bytes(len);
        self
    }

    /// The value's CBOR bytes: a map of its fields in the order of their keys.
    pub fn bytes(&self) -> Vec<u8> {
        let mut bytes = head(5, 5 + self.later.len() as u64);
        for (key, item) in [
            (1, byte_item(self.device.as_bytes())),
            (2, byte_item(self.channel.as_bytes())),
            (3, uint(self.epoch)),
            (4, byte_item(&self.nonce)),
            (5, byte_item(&self.ct)),
        ]
        .into_iter()
        .chain(self.later.iter().cloned())
        {
            bytes.extend(uint(key));
            bytes.extend(item);
        }
        bytes
    }
}

/// A channel whose presence the tests upload and read, and its group's current epoch.
#[derive(Debug, Clone, Copy)]
pub struct Channel {
    pub space: SpaceId,
    pub id: ChannelId,
    pub epoch: u64,
}

impl Channel {
    /// A value of `device` in the channel under its epoch, as [`Value::new`] makes it.
    pub fn value(&self, device: DeviceId) -> Value {
        Value::new(device, self.id, self.epoch)
    }

    /// The channel with its group in `epoch`.
    pub fn at(self, epoch: u64) -> Self {
        Channel { epoch, ..self }
    }
}

/// The scene of the Delivery Service's tests with its channel's group formed: alice's, bob's and
/// carol's laptops are its members, dave's is a channel member outside it, eve's is outside the
/// space. GitHub user IDs from `github` on.
pub fn formed(github: u64) -> (Scene, Channel) {
    let (s, _) = Scene::formed(github);
    let channel = Channel {
        space: s.fixture.team.space,
        id: s.fixture.general,
        epoch: epoch(&s),
    };
    (s, channel)
}

/// A device's requests for presence to one instance of the server, with a bearer token: a device
/// session's or a service token's.
pub struct Caller {
    /// Who makes them, as the checks name it.
    pub who: String,
    base: String,
    token: String,
    https: Https,
}

impl Caller {
    /// The requests of `person`'s laptop on `person`'s session to `flow`'s server.
    pub fn of(flow: &DeviceFlow, person: &Person) -> Self {
        let token = person.session.token();
        let mut caller = Self::on(&flow.stand.server, flow, token, &person.laptop);
        caller.who = format!("{}'s laptop", person.handle);
        caller
    }

    /// The requests of `device` on the bearer token `token` to `server`, an instance of `flow`'s
    /// server.
    pub fn on(server: &Server, flow: &DeviceFlow, token: &str, device: &Device) -> Self {
        Caller {
            who: device.name.clone(),
            base: server.base().to_owned(),
            token: token.to_owned(),
            https: Https::on_session(server, flow, token, device),
        }
    }

    /// `<method> <path>` of `channel`'s presence, `method` `get` or `put`, with the CBOR `body`
    /// for `put`, stating the version, with the bearer token and, if `proved`, the device's proof
    /// on a new challenge; checked against the spec.
    fn call(&self, method: &str, channel: &Channel, body: &[u8], proved: bool) -> Response {
        let path = presence_path(channel.space, channel.id);
        let url = format!("{}{path}", self.base);
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        let bearer = format!("Bearer {}", self.token);
        let proof = if proved {
            self.https.proof()
        } else {
            String::new()
        };
        let mut headers = vec![
            (VERSION_HEADER, version.as_str()),
            ("authorization", &bearer),
        ];
        if proved {
            headers.push((PROOF, &proof));
        }
        let response = if method == "get" {
            get_url(&url, &headers)
        } else {
            headers.push(("content-type", CBOR));
            send(method, &url, &headers, body)
        };
        conforms(method, &path, &response);
        response
    }

    /// `PUT` of `bytes` as the device's value in `channel`, proved if `proved`.
    pub fn put_raw(&self, channel: &Channel, bytes: &[u8], proved: bool) -> Response {
        self.call("put", channel, bytes, proved)
    }

    /// `PUT` of `value` in `channel`, proved.
    pub fn put(&self, channel: &Channel, value: &Value) -> Response {
        self.put_raw(channel, &value.bytes(), true)
    }

    /// `GET` of `channel`'s values, proved if `proved`.
    pub fn get(&self, channel: &Channel, proved: bool) -> Response {
        self.call("get", channel, &[], proved)
    }

    /// The values of `channel` the device gets, proved: their bytes, sorted.
    #[track_caller]
    pub fn values(&self, channel: &Channel) -> Vec<Vec<u8>> {
        values_of(&self.get(channel, true))
    }
}

/// Checks that each of `callers` gets exactly `expected` of `channel`'s values, byte for byte;
/// `what` names the case.
#[track_caller]
pub fn assert_given(callers: &[&Caller], channel: &Channel, expected: &[&Value], what: &str) {
    let expected = sorted(expected);
    for caller in callers {
        let found = caller.values(channel);
        assert!(
            found == expected,
            "{what}: {} gets {} values, not the {} expected",
            caller.who,
            found.len(),
            expected.len()
        );
    }
}

/// Checks that the server keeps each of `uploads`, a caller and its value, in `channel`.
#[track_caller]
pub fn assert_all_kept(channel: &Channel, uploads: &[(&Caller, &Value)]) {
    for (caller, value) in uploads {
        let what = format!("{}'s value", caller.who);
        assert_kept(&caller.put(channel, value), &what);
    }
}

/// Checks that `response` to `<method> <path>` conforms to the spec.
#[track_caller]
fn conforms(method: &str, path: &str, response: &Response) {
    let violations = operation_violations(the_spec(), method, &template(path), response);
    assert_eq!(
        violations,
        Vec::<String>::new(),
        "{method} {path}: {}",
        text(response)
    );
}

/// The values in `response`, `200` with `{"values": [<bytes>]}`: their bytes, sorted.
#[track_caller]
pub fn values_of(response: &Response) -> Vec<Vec<u8>> {
    assert_eq!(response.status, 200, "the values: {}", text(response));
    let body = json(response);
    let items = body["values"]
        .as_array()
        .unwrap_or_else(|| panic!("no values in {body}"));
    let mut values: Vec<Vec<u8>> = (items.iter())
        .map(|item| unb64(item.as_str().unwrap_or_else(|| panic!("bytes, not {item}"))))
        .collect();
    values.sort();
    values
}

/// `values`' bytes, sorted.
pub fn sorted(values: &[&Value]) -> Vec<Vec<u8>> {
    let mut bytes: Vec<Vec<u8>> = values.iter().map(|value| value.bytes()).collect();
    bytes.sort();
    bytes
}

/// Checks that `response` takes an upload.
#[track_caller]
pub fn assert_kept(response: &Response, what: &str) {
    assert!(
        (200..300).contains(&response.status),
        "{what}: kept, not {}",
        text(response)
    );
}

/// Checks that `response` refuses with a `4xx` and a problem details object, of the problem `name`
/// if given.
#[track_caller]
pub fn assert_refused_with(response: &Response, name: Option<&str>, what: &str) {
    assert!(
        (400..500).contains(&response.status),
        "{what}: refused, not {}",
        text(response)
    );
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{what}: a problem, not {:?}: {}",
        response.content_type,
        text(response)
    );
    if let Some(name) = name {
        assert_eq!(
            json(response)["type"],
            problem(name),
            "{what}: {}",
            text(response)
        );
    }
}

/// Checks that `response` refuses a request without the device's proof: `401`, `auth-required`.
#[track_caller]
pub fn assert_unproved(response: &Response, what: &str) {
    assert_eq!(response.status, 401, "{what}: {}", text(response));
    assert_refused_with(response, Some(AUTH_REQUIRED), what);
}

/// Whether a row of `db` holds `bytes`.
pub fn held(db: &TestDb, bytes: &[u8]) -> bool {
    let hex = hex(bytes);
    db.rows().iter().any(|row| row.contains(&hex))
}

/// Checks that no row of `db` holds `value`: the server keeps nothing of it.
#[track_caller]
pub fn assert_not_held(db: &TestDb, value: &Value, what: &str) {
    assert!(!held(db, &value.bytes()), "{what}: the value is kept");
}

/// Checks that no row of `db` holds `value` within [`GONE_WITHIN`]: the server deleted it.
#[track_caller]
pub fn assert_gone(db: &TestDb, value: &Value, what: &str) {
    let deadline = Instant::now() + GONE_WITHIN;
    let bytes = value.bytes();
    while held(db, &bytes) {
        assert!(
            Instant::now() < deadline,
            "{what}: still kept after {GONE_WITHIN:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The scene's people's requests: alice's, bob's, carol's, dave's and eve's laptops.
pub fn callers(s: &Scene) -> [Caller; 5] {
    let f = &s.fixture;
    let flow = &f.team.flow;
    [
        &f.team.alice,
        &f.team.bob,
        &f.team.carol,
        &f.team.dave,
        &f.eve,
    ]
    .map(|person| Caller::of(flow, person))
}

/// The epoch of the scene's channel, as alice's laptop holds it.
#[track_caller]
pub fn epoch(s: &Scene) -> u64 {
    (s.alice.mls.epoch(s.fixture.general)).expect("alice's epoch")
}
