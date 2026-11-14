//! People whose machines sign in to a server behind the relay, the spaces their machines create,
//! revisions the tests sign with their devices' keys and store through the API, and what the CLI
//! shows of a space: for the tests of a space's roster revisions in the client.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use skald_crypto::SigningKey;
use skald_github_stand_in::User;
use skald_proto::{
    AccountId, Cbor, Cert, DeviceId, Member, Name, NonEmpty, Role, Roster, RosterBody, SpaceId,
    Unknown,
};

use super::browser::Browser;
use super::cloud::Cloud;
use super::keys;
use super::machine::{Finished, Machine};

/// The commands that create a space, list the device's spaces and show a space's members.
pub const CREATE_SPACE: &str = "create-space";
pub const SPACES: &str = "spaces";
pub const MEMBERS: &str = "members";

/// The requests for the account's personal space and for its spaces, as their lines start.
pub const PERSONAL_REQUEST: &str = "POST /account/personal-space ";
pub const LIST_REQUEST: &str = "GET /spaces ";

/// The roles as the CLI shows them.
pub const ADMIN: &str = "admin";
pub const MEMBER: &str = "member";

/// What a warning says.
pub const WARNING: &str = "warning";

/// A person's account and its first machine, signed in, which holds a valid certificate.
pub struct Person {
    pub user: User,
    pub machine: Machine,
    /// The machine's session token.
    pub token: String,
    pub account: AccountId,
    pub master: [u8; 32],
    /// The account's username, its handle in the spaces it is in.
    pub handle: Name,
    /// The machine's device.
    pub device: DeviceId,
    /// The machine's host name, its device certificate's `name`.
    pub host: &'static str,
    /// The device's signing key, as its secrets file holds it.
    pub key: SigningKey,
}

impl Person {
    /// The person's entry in a roster, with `role`.
    pub fn member(&self, role: Role) -> Member {
        Member {
            account: self.account,
            handle: self.handle.clone(),
            role,
            master: self.master,
            invite: None,
            excluded: None,
            unknown: Unknown::default(),
        }
    }
}

/// A server whose codes live for 2 minutes and poll every second, behind the relay, and a browser
/// for the people who approve their machines' codes.
pub struct World {
    pub cloud: Cloud,
    pub browser: Browser,
}

impl World {
    pub fn start() -> Self {
        World {
            cloud: Cloud::start(120, 1),
            browser: Browser::launch(),
        }
    }

    /// A new account of GitHub user `id` with `login`, whose first machine, on the host `host`,
    /// signs in.
    pub fn sign_up(&self, id: u64, login: &str, host: &'static str) -> Person {
        let user = self.cloud.user(id, login);
        let (machine, signed_in) = self.cloud.first_machine(&self.browser.page(), &user, host);
        assert!(signed_in.success, "{signed_in:?}");
        self.person(user, machine, host)
    }

    /// The person of GitHub user `user` whose first machine, on the host `host`, holds the session
    /// the server gave last and a valid certificate.
    pub fn person(&self, user: User, machine: Machine, host: &'static str) -> Person {
        let cloud = &self.cloud;
        let token = cloud.last_token();
        let (status, session) = cloud.session_of(&token);
        assert_eq!(status, 200, "GET /session: {session}");
        let handle: Name = session["username"]
            .as_str()
            .and_then(|name| name.parse().ok())
            .unwrap_or_else(|| panic!("a username that is a valid name: {session}"));
        let devices = cloud.devices(&token);
        assert_eq!(devices.len(), 1, "the account's devices: {devices:?}");
        let cert = devices[0]
            .cert
            .clone()
            .expect("the first device's certificate");
        let key = keys::signing_key(&machine.files_holding(&token), &cert.body().signing);
        Person {
            account: cloud.account_of(&token),
            master: cloud.master(&token).expect("the account's master key"),
            user,
            machine,
            token,
            handle,
            device: devices[0].id,
            host,
            key,
        }
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after 1970")
        .as_secs()
}

/// Runs `skald` with `args` on `machine`.
pub fn run(machine: &Machine, args: &[&str]) -> Finished {
    machine.start(args).finish()
}

/// `person`'s machine creates the space `name`; gives its revision 1, as the server holds it.
pub fn create(cloud: &Cloud, person: &Person, name: &str) -> Roster {
    let created = run(&person.machine, &[CREATE_SPACE, name]);
    assert!(created.success, "{name}: {created:?}");
    let (status, body) = cloud.get_as("/spaces", &person.token);
    assert_eq!(status, 200, "GET /spaces: {body}");
    let space: SpaceId = body["spaces"]
        .as_array()
        .and_then(|spaces| spaces.iter().find(|space| space["name"] == json!(name)))
        .and_then(|space| space["space"].as_str())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("the server has no space {name}: {body}"));
    let revisions = cloud.revisions(&person.token, space);
    assert_eq!(revisions.len(), 1, "{name}: {revisions:?}");
    revisions[0].1.clone()
}

/// From now on the relay gives `space`, named `name`, as the account's personal space, as a
/// compromised server would.
pub fn give_as_personal(cloud: &Cloud, space: SpaceId, name: &str) {
    let answer = json!({ "space": space.to_string(), "name": name }).to_string();
    cloud
        .relay
        .rewrite(PERSONAL_REQUEST, move |_| answer.clone());
}

/// From now on the relay marks `space`, and no other, as personal in the account's spaces, as a
/// compromised server would.
pub fn mark_as_personal(cloud: &Cloud, space: SpaceId) {
    let marked = space.to_string();
    cloud.relay.rewrite(LIST_REQUEST, move |body| {
        let mut list: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        for space in list["spaces"].as_array_mut().into_iter().flatten() {
            space["personal"] = json!(space["space"] == json!(marked));
        }
        list.to_string()
    });
}

/// The revision after `previous`, signed by `signer` with `key`, with the members `members`
/// gives.
pub fn next(
    previous: &Roster,
    (signer, key): (DeviceId, &SigningKey),
    members: impl FnOnce(&mut Vec<Member>),
) -> Roster {
    next_with(previous, (signer, key), |body| {
        let mut list = body.members.clone().into_vec();
        members(&mut list);
        body.members = NonEmpty::from_vec(list).expect("a member");
    })
}

/// The revision after `previous`, signed by `signer` with `key`, with the changes `change` makes
/// to its body.
pub fn next_with(
    previous: &Roster,
    (signer, key): (DeviceId, &SigningKey),
    change: impl FnOnce(&mut RosterBody),
) -> Roster {
    let mut body: RosterBody = previous.body().clone();
    body.revision += 1;
    body.prev = Some(previous.hash());
    body.signer = signer;
    body.timestamp = now();
    change(&mut body);
    skald_crypto::sign_roster(&body, key).expect("a revision")
}

/// The revision after `previous`, signed by `person`'s device.
pub fn next_by(
    previous: &Roster,
    person: &Person,
    members: impl FnOnce(&mut Vec<Member>),
) -> Roster {
    next(previous, (person.device, &person.key), members)
}

/// Sets the role of `account` in `members` to `role`.
pub fn set_role(members: &mut [Member], account: AccountId, role: Role) {
    members
        .iter_mut()
        .find(|member| member.account == account)
        .expect("a member")
        .role = role;
}

/// Sends `POST <path>` with the JSON `request`, or without a body if `None`, as the session
/// `token`, through the API: the status and the body.
pub fn post_as(cloud: &Cloud, token: &str, path: &str, request: Option<&Value>) -> (u16, Value) {
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let post = agent
        .post(format!("{}{path}", cloud.url))
        .header(
            skald_proto::VERSION_HEADER,
            skald_proto::PROTOCOL_VERSION.to_string(),
        )
        .header("authorization", format!("Bearer {token}"));
    let sent = match request {
        Some(request) => post
            .header("content-type", "application/json")
            .send(request.to_string().as_bytes()),
        None => post.send_empty(),
    };
    let mut response = sent.unwrap_or_else(|err| panic!("POST {path}: {err}"));
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&body).unwrap_or(Value::Null))
}

/// Stores `roster` on the server as the session `token`, through the API; fails unless the server
/// stores it.
pub fn store(cloud: &Cloud, token: &str, roster: &Roster) {
    let space = roster.body().space;
    let path = format!("/spaces/{space}/revisions");
    let request = json!({ "revision": URL_SAFE_NO_PAD.encode(roster.to_cbor()) });
    let (status, body) = post_as(cloud, token, &path, Some(&request));
    assert_eq!(
        status,
        201,
        "the server did not store revision {}: {body}",
        roster.body().revision
    );
}

/// The request for `space`'s revisions, as its line starts.
pub fn revisions_request(space: SpaceId) -> String {
    format!("GET /spaces/{space}/revisions")
}

/// The certificates of the devices of `space`'s accounts, as the server gives them to the session
/// `token`, straight from the server.
pub fn space_certs(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<Cert> {
    let path = format!("/spaces/{space}/devices");
    let (status, body) = cloud.get_as(&path, token);
    assert_eq!(status, 200, "GET {path}: {body}");
    body["devices"]
        .as_array()
        .unwrap_or_else(|| panic!("GET {path}: {body}"))
        .iter()
        .filter_map(|device| device["cert"].as_str())
        .map(|cert| {
            let bytes = URL_SAFE_NO_PAD
                .decode(cert)
                .expect("a certificate in base64url");
            Cert::from_cbor(&bytes).expect("a certificate")
        })
        .collect()
}

/// The words of `line`: its runs of ASCII letters, digits and `-`.
pub fn words(line: &str) -> Vec<&str> {
    line.split(|char: char| !(char.is_ascii_alphanumeric() || char == '-'))
        .filter(|word| !word.is_empty())
        .collect()
}

/// The lines of `finished` that are no warnings, on either output.
pub fn plain_lines(finished: &Finished) -> Vec<String> {
    finished
        .shown()
        .lines()
        .filter(|line| !line.to_lowercase().contains(WARNING))
        .map(str::to_owned)
        .collect()
}

/// The warning lines of `finished`, on either output, in lowercase.
pub fn warnings(finished: &Finished) -> Vec<String> {
    finished
        .shown()
        .lines()
        .map(str::to_lowercase)
        .filter(|line| line.contains(WARNING))
        .collect()
}

/// Whether a line of `lines` holds the word `word`.
pub fn holds_word(lines: &[String], word: &str) -> bool {
    lines.iter().any(|line| words(line).contains(&word))
}

/// Runs `skald members <space>` on `machine`; fails unless it succeeds.
pub fn members(machine: &Machine, space: &str) -> Finished {
    let shown = run(machine, &[MEMBERS, space]);
    assert!(shown.success, "{shown:?}");
    shown
}

/// Fails unless `shown` shows `handle` with the role `role` and never with the other role.
pub fn assert_role(shown: &Finished, handle: &Name, role: &str) {
    let other = if role == ADMIN { MEMBER } else { ADMIN };
    let lines = plain_lines(shown);
    let of_handle: Vec<&String> = lines
        .iter()
        .filter(|line| words(line).contains(&handle.as_str()))
        .collect();
    assert!(
        of_handle.iter().any(|line| words(line).contains(&role)),
        "{handle} is not shown as {role}: {shown:?}"
    );
    assert!(
        !of_handle.iter().any(|line| words(line).contains(&other)),
        "{handle} is shown as {other}: {shown:?}"
    );
}
