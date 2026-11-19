//! What the tests share: the commands and their output, Ada's account with its browser kept, the
//! space `acme` with Bob as a member, the service agent as the server holds it, its tokens through
//! the API, and what a refused command must leave as it was.

use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::{check_valid_device, verify};
use skald_proto::{
    Cbor as _, Cert, ChannelId, DeviceId, DeviceKind, GRANT_LABEL, Grant, Role, Roster, SpaceId,
};

use crate::browser::Page;
use crate::cloud::{Cloud, agent, get};
use crate::machine::{Finished, Machine};
use crate::world::{ACME, HOST, Person, World, create, next_by, now, run, store, store_request};

/// The commands: create a service agent; a new token for one; list a space's tokens; revoke one.
pub const CREATE_SERVICE_AGENT: &str = "create-service-agent";
pub const CREATE_SERVICE_TOKEN: &str = "create-service-token";
pub const SERVICE_TOKENS: &str = "service-tokens";
pub const REVOKE_SERVICE_TOKEN: &str = "revoke-service-token";

/// Their options: a channel of the grant, once per channel; the grant's expiry; a permission of
/// the token, once per permission; keeping the agent's earlier tokens.
pub const CHANNEL: &str = "--channel";
pub const EXPIRES: &str = "--expires";
pub const PERMISSION: &str = "--permission";
pub const KEEP_EARLIER: &str = "--keep-earlier";

/// The environment variables of the two secrets, as stdout names them.
pub const KEY_VARIABLE: &str = "SKALD_SERVICE_KEY";
pub const TOKEN_VARIABLE: &str = "SKALD_SERVICE_TOKEN";

/// The service agent the tests create, its owner and its agent.
pub const BOT: &str = "ci/deploy-bot";
pub const OWNER: &str = "ci";
pub const AGENT_NAME: &str = "deploy-bot";

/// The permissions of a token.
pub const SEND: &str = "send";
pub const RECEIVE: &str = "receive";

/// A day, in seconds; the grant's default expiry, in days.
pub const DAY: u64 = 86_400;
pub const DEFAULT_DAYS: u64 = 90;

/// The web session's cookie.
pub const SESSION_COOKIE: &str = "skald_session";

/// A new account of GitHub user `id` with `login`, whose first machine, on the host `host`, signs
/// in with `page`, the person's browser, which keeps its web session.
pub fn sign_up_on(
    world: &World,
    page: &Page,
    (id, login): (u64, &str),
    host: &'static str,
) -> Person {
    let user = world.cloud.user(id, login);
    let (machine, signed_in) = world.cloud.first_machine(page, &user, host);
    assert!(signed_in.success, "{signed_in:?}");
    world.person(user, machine, host)
}

/// The space `acme` Ada's machine creates: its ID, its default channel and its revisions as the
/// server holds them.
pub struct Acme {
    pub id: SpaceId,
    pub default: ChannelId,
    pub revisions: Vec<Roster>,
}

impl Acme {
    /// `ada`'s machine creates `acme`, with revision 2 adding its default channel.
    pub fn create(cloud: &Cloud, ada: &Person) -> Self {
        let second = create(cloud, ada, ACME);
        let (id, default) = (second.body().space, second.body().channels[0].id);
        Acme {
            id,
            default,
            revisions: rosters(cloud, &ada.token, id),
        }
    }

    /// As [`Acme::create`], then revision 3, which Ada's device signs, adds `bob` as a member.
    pub fn with_member(cloud: &Cloud, ada: &Person, bob: &Person) -> Self {
        let mut acme = Self::create(cloud, ada);
        let third = next_by(acme.latest(), ada, |members| {
            members.push(bob.member(Role::Member));
        });
        store(cloud, &ada.token, &third);
        acme.revisions.push(third);
        acme
    }

    pub fn latest(&self) -> &Roster {
        self.revisions.last().expect("a revision")
    }
}

/// The revisions of `space` the server holds, for the session `token`.
pub fn rosters(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Roster> {
    let held = cloud.revisions(token, space);
    held.into_iter().map(|revision| revision.1).collect()
}

/// The device of the service agent `owner/agent` in `roster`, if it has the entry.
pub fn entry_of(roster: &Roster, address: &str) -> Option<DeviceId> {
    let (owner, agent) = address.split_once('/').expect("owner/agent");
    roster
        .body()
        .service_agents
        .iter()
        .find(|entry| entry.owner.as_str() == owner && entry.agent.as_str() == agent)
        .map(|entry| entry.device)
}

/// Runs `skald create-service-agent <space> <address> --channel <channel>... <extra>` on `machine`.
pub fn create_agent(
    machine: &Machine,
    space: &str,
    address: &str,
    channels: &[ChannelId],
    extra: &[&str],
) -> Finished {
    let channels: Vec<String> = channels.iter().map(ToString::to_string).collect();
    let mut args = vec![CREATE_SERVICE_AGENT, space, address];
    for channel in &channels {
        args.extend([CHANNEL, channel.as_str()]);
    }
    args.extend_from_slice(extra);
    run(machine, &args)
}

/// The two secrets a creation gave.
#[derive(Debug, Clone)]
pub struct Secrets {
    /// The key secret.
    pub key: String,
    /// The token.
    pub token: String,
}

/// The values of stdout's lines `<name>=<value>`, by name, in their order; fails on any other
/// line, a name twice or an empty value.
#[track_caller]
fn assignments(finished: &Finished) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for line in finished.stdout.lines() {
        let (name, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("stdout holds more than the secrets: {finished:?}"));
        assert!(
            !value.is_empty() && !value.contains(char::is_whitespace),
            "{name}: not a single value: {finished:?}"
        );
        assert!(
            found.iter().all(|(seen, _)| seen != name),
            "{name} twice: {finished:?}"
        );
        found.push((name.to_owned(), value.to_owned()));
    }
    found
}

/// The key secret and the token of `finished`, a creation that must succeed: stdout holds them as
/// the lines `SKALD_SERVICE_KEY=<key secret>` and `SKALD_SERVICE_TOKEN=<token>`, and nothing else.
#[track_caller]
pub fn secrets_of(finished: &Finished) -> Secrets {
    assert!(finished.success, "the creation failed: {finished:?}");
    let found = assignments(finished);
    let value = |name: &str| {
        found
            .iter()
            .find(|(seen, _)| seen == name)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("no {name} on stdout: {finished:?}"))
    };
    assert_eq!(
        found.len(),
        2,
        "stdout holds more than the secrets: {finished:?}"
    );
    Secrets {
        key: value(KEY_VARIABLE),
        token: value(TOKEN_VARIABLE),
    }
}

/// The token of `finished`, a new token's run that must succeed: stdout holds it as the line
/// `SKALD_SERVICE_TOKEN=<token>`, and nothing else.
#[track_caller]
pub fn token_of(finished: &Finished) -> String {
    assert!(finished.success, "the new token failed: {finished:?}");
    let found = assignments(finished);
    match found.as_slice() {
        [(name, token)] if name == TOKEN_VARIABLE => token.clone(),
        _ => panic!("stdout holds other than the token: {finished:?}"),
    }
}

/// The `service` device of `person`'s account and its certificate, as the server gives them: the
/// one device of the account beside its first.
#[track_caller]
pub fn service_of(cloud: &Cloud, person: &Person) -> (DeviceId, Cert) {
    let others: Vec<_> = cloud
        .devices(&person.token)
        .into_iter()
        .filter(|device| device.id != person.device)
        .collect();
    assert_eq!(
        others.len(),
        1,
        "the account's devices beside its first: {others:?}"
    );
    let device = others.into_iter().next().expect("a device");
    let cert = device
        .cert
        .unwrap_or_else(|| panic!("the service device {} has no certificate", device.id));
    (device.id, cert)
}

/// Fails unless `cert`, among `certs`, is a valid certificate of a `service` device on `person`'s
/// account, against its master key, issued by its first device.
#[track_caller]
pub fn assert_certified_by(person: &Person, certs: &[Cert], device: DeviceId) {
    let cert = check_valid_device(&person.master, certs, &[], device)
        .unwrap_or_else(|err| panic!("the service device's certificate is not valid: {err:?}"));
    let body = cert.body();
    assert_eq!(body.kind, DeviceKind::Service, "{body:?}");
    assert_eq!(body.account, person.account, "{body:?}");
    assert_eq!(body.issuer, Some(person.device), "{body:?}");
}

/// A token's own record and grant, `GET /service-token` with it, if the server takes it; fails
/// unless it answers `200` or `401`.
#[track_caller]
pub fn own_record(cloud: &Cloud, token: &str) -> Option<Value> {
    match cloud.get_as("/service-token", token) {
        (200, body) => Some(body),
        (401, _) => None,
        (status, body) => panic!("GET /service-token: {status} {body}"),
    }
}

/// The grant in `record`, a token's own record.
#[track_caller]
pub fn grant_of(record: &Value) -> Grant {
    let bytes = record["grant"]
        .as_str()
        .map(|grant| URL_SAFE_NO_PAD.decode(grant).expect("a grant in base64url"))
        .unwrap_or_else(|| panic!("no grant in {record}"));
    Grant::from_cbor(&bytes).unwrap_or_else(|err| panic!("a grant that does not decode: {err}"))
}

/// The strings of `value`, an array of them, sorted.
#[track_caller]
pub fn sorted(value: &Value) -> Vec<String> {
    let mut strings: Vec<String> = serde_json::from_value(value.clone())
        .unwrap_or_else(|err| panic!("not an array of strings, {err}: {value}"));
    strings.sort();
    strings
}

/// `channels` as text, sorted.
pub fn sorted_ids(channels: &[ChannelId]) -> Vec<String> {
    let mut ids: Vec<String> = channels.iter().map(ToString::to_string).collect();
    ids.sort();
    ids
}

/// Fails unless `grant` is a grant of `device` for `channels`, signed by `signer`'s first device,
/// whose signature verifies with that device's key, expiring `days` days from now, give or take a
/// day: so it passes the recipients' checks of "Service grant" while it lasts.
#[track_caller]
pub fn assert_grant(
    grant: &Grant,
    device: DeviceId,
    channels: &[ChannelId],
    signer: &Person,
    days: u64,
) {
    let body = &grant.body;
    assert_eq!(body.service, device, "the grant's service: {body:?}");
    let listed: Vec<ChannelId> = body.channels.iter().copied().collect();
    assert_eq!(
        sorted_ids(&listed),
        sorted_ids(channels),
        "the grant's channels"
    );
    assert_eq!(body.signer, signer.device, "the grant's signer: {body:?}");
    verify(
        &signer.key.public_key(),
        GRANT_LABEL,
        &[&body.to_cbor()],
        &grant.signature,
    )
    .unwrap_or_else(|_| panic!("the grant's signature does not verify: {body:?}"));
    let expected = now() + days * DAY;
    assert!(
        body.expires.abs_diff(expected) <= DAY,
        "the grant expires at {}, not {days} days from now, {expected}",
        body.expires
    );
}

/// Fails unless `record`, a token's record, is of the service agent `ci/deploy-bot` on `device`,
/// with `permissions` and `channels`.
#[track_caller]
pub fn assert_record(
    record: &Value,
    device: DeviceId,
    permissions: &[&str],
    channels: &[ChannelId],
) {
    assert_eq!(record["device"], device.to_string(), "{record}");
    assert_eq!(
        (&record["owner"], &record["agent"]),
        (&Value::from(OWNER), &Value::from(AGENT_NAME)),
        "{record}"
    );
    let mut wanted: Vec<String> = permissions.iter().map(|p| (*p).to_owned()).collect();
    wanted.sort();
    assert_eq!(sorted(&record["permissions"]), wanted, "{record}");
    assert_eq!(
        sorted(&record["channels"]),
        sorted_ids(channels),
        "{record}"
    );
}

/// The ID in `record`, a token's record.
#[track_caller]
pub fn id_of(record: &Value) -> String {
    record["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no ID in {record}"))
        .to_owned()
}

/// The IDs of the tokens of `space` the server lists to `person`'s session, sorted.
#[track_caller]
pub fn listed_ids(cloud: &Cloud, person: &Person, space: SpaceId) -> Vec<String> {
    let path = format!("/spaces/{space}/service-tokens");
    let (status, body) = cloud.get_as(&path, &person.token);
    assert_eq!(status, 200, "GET {path}: {body}");
    let mut ids: Vec<String> = body["tokens"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .map(id_of)
        .collect();
    ids.sort();
    ids
}

/// Runs `skald service-tokens acme` on `machine`, which must succeed, and gives the lines that
/// list each of `ids`: fails unless each holds `ci/deploy-bot`.
#[track_caller]
pub fn cli_lines_of(machine: &Machine, ids: &[&str]) -> Vec<Option<String>> {
    let listed = run(machine, &[SERVICE_TOKENS, ACME]);
    assert!(listed.success, "{listed:?}");
    ids.iter()
        .map(|id| {
            let line = listed.stdout.lines().find(|line| line.contains(id))?;
            assert!(
                line.contains(BOT),
                "{id} is not listed as {BOT}: {listed:?}"
            );
            Some(line.to_owned())
        })
        .collect()
}

/// The `device_name` of each device session of the web session of `page`, the person's browser.
#[track_caller]
pub fn device_session_names(cloud: &Cloud, page: &Page) -> Vec<Option<String>> {
    let cookies = page.cookies_of(HOST);
    let cookie = cookies
        .iter()
        .find(|cookie| cookie.name == SESSION_COOKIE)
        .unwrap_or_else(|| panic!("no web session: {cookies:?}"))
        .pair();
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let url = format!("{}/account/sessions", cloud.url);
    let mut answer = get(|| {
        agent()
            .get(&url)
            .header(skald_proto::VERSION_HEADER, &version)
            .header("cookie", &cookie)
            .call()
    })
    .unwrap_or_else(|err| panic!("GET {url}: {err}"));
    assert_eq!(answer.status().as_u16(), 200, "GET {url}");
    let text = answer
        .body_mut()
        .read_to_string()
        .expect("read the sessions");
    let body: Value = serde_json::from_str(&text).unwrap_or_else(|err| panic!("{err}: {text}"));
    body["sessions"]
        .as_array()
        .unwrap_or_else(|| panic!("GET /account/sessions: {body}"))
        .iter()
        .filter(|session| session["kind"] == "device")
        .map(|session| session["device_name"].as_str().map(str::to_owned))
        .collect()
}

/// The requests that register a device, store its certificate, store a revision of `space`, or
/// create or revoke a token of it, as their lines start.
pub fn writes(space: SpaceId) -> [String; 5] {
    [
        "POST /devices ".to_owned(),
        "PUT /devices/".to_owned(),
        store_request(space),
        format!("POST /spaces/{space}/service-tokens"),
        format!("DELETE /spaces/{space}/service-tokens"),
    ]
}

/// How many of the requests of [`writes`] the machines sent from `since` on.
pub fn writes_since(cloud: &Cloud, since: Instant, space: SpaceId) -> usize {
    writes(space)
        .iter()
        .map(|request| cloud.count_sent(since, request))
        .sum()
}

/// What a refused command leaves as it was: the devices of an account, with whether each has a
/// certificate, the revisions of a space, and its tokens.
#[derive(Debug, PartialEq)]
pub struct Snapshot {
    devices: Vec<(DeviceId, bool)>,
    revisions: Vec<Roster>,
    tokens: Vec<String>,
}

impl Snapshot {
    /// The devices of `person`'s account, and the revisions and tokens of `space` as the server
    /// gives them to `admin`, an admin of it.
    pub fn take(cloud: &Cloud, person: &Person, admin: &Person, space: SpaceId) -> Self {
        let mut devices: Vec<(DeviceId, bool)> = cloud
            .devices(&person.token)
            .into_iter()
            .map(|device| (device.id, device.cert.is_some()))
            .collect();
        devices.sort();
        Snapshot {
            devices,
            revisions: rosters(cloud, &admin.token, space),
            tokens: listed_ids(cloud, admin, space),
        }
    }
}

/// Fails unless `finished` failed, printed nothing on stdout, and says `why`, in any case.
#[track_caller]
pub fn assert_refused_saying(finished: &Finished, why: &str) {
    assert!(!finished.success, "{finished:?}");
    assert_eq!(finished.stdout, "", "a refused run printed: {finished:?}");
    assert!(
        finished.shown().to_lowercase().contains(why),
        "it does not say {why:?}: {finished:?}"
    );
}
