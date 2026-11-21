//! Checking certificates' SSH signatures against GitHub (`docs/protocol/membership.md`, "Device
//! certificate" and "Device checks"; `docs/dev/architecture.md`, "Network").
//!
//! - A client takes a GitHub user's login and public SSH keys, authentication and signing keys,
//!   from GitHub's API, unauthenticated, over HTTPS through `HTTPS_PROXY` as it reaches its server.
//!   A hidden setting, [`GITHUB_API`], names the API's address; the results from any address but
//!   GitHub's own are kept apart, so a run with another one, as an agent could make, changes
//!   nothing later runs show.
//! - It checks the `ssh` of each certificate it takes, of its own account's devices and of its
//!   spaces' members', once, and keeps the result: passed, with the login GitHub gave, or failed.
//!   One it cannot check, GitHub's rate limit included, waits for a later run. Failing to reach
//!   GitHub fails no command.
//! - `skald devices` and `skald members` show the GitHub login of each device whose signature
//!   passed. One that failed warns of a key change, once; the device stays as its chain makes it.
//!
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay; a test plays the person on the verification page in headless Chrome. Ada's
//! machine signs in, and so, in most tests, does Bob's, which creates `acme`, and a revision his
//! device signs adds Ada as a member, as the tests of roster revisions do
//! (`roster_revisions/main.rs`). Ada's laptop is a device the tests write to the server's database
//! on her account, certified by her machine's device with the keys its secrets file holds, as the
//! tests of sending set devices up (`sending/fixtures.rs`); its certificate names Ada's GitHub user
//! and carries an SSH signature the tests make with an Ed25519 key of their own. No run of either
//! machine sees the laptop before the tests set the GitHub API's address on it.
//!
//! GitHub's API is a GitHub stand-in of its own, apart from the one the server signs in with, in
//! which Ada's GitHub user has the login [`LOGIN`], shown nowhere else. As GitHub does, it serves a
//! user by ID and the user's SSH keys, authentication and signing keys. The tests set them, and its
//! faults, through its control endpoints, as a test in another process would:
//! - `PUT /_stand-in/ssh-keys`, `{"user": <id>, "authentication": [<key>, …], "signing": [<key>,
//!   …]}`, each key an OpenSSH public key line: the user's keys from then on;
//! - `PUT /_stand-in/faults` with `{"fault": "api_status", "status": <status>}`: those endpoints
//!   answer with that status; with `{"fault": "rate_limited"}`: they answer as GitHub at its rate
//!   limit, `403` with `x-ratelimit-remaining: 0`.
//!
//! `GitHub::requests` holds the requests to those endpoints too. A gate before the stand-in drops
//! connections while GitHub cannot be reached ([`gate`]).
//!
//! The CLI, as these tests use it, beside `skald devices` (`device_list/main.rs`) and `skald members
//! <space>` (`roster_revisions/main.rs`):
//! - a device it trusts shows on a line that is no warning, holding the device's ID and its
//!   certificate's `name`; in `skald devices`, above the line that says `not linked`;
//! - the GitHub login of a device whose signature passed shows on that line, as a word;
//! - the warning of a key change: a line saying `warning` that holds the device's ID.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod agents;
mod failing;
mod gate;
mod passing;
mod proxied;
mod unchecked;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
// The TLS endpoint and test CA of the tests of HTTPS, and the recording proxy of the tests behind
// a proxy.
#[path = "../https_proxy/proxy.rs"]
mod proxy;
#[path = "../common/tls.rs"]
mod tls;
#[path = "../https/front.rs"]
mod tls_front;

use std::process::Stdio;

use serde_json::{Value, json};
use skald_crypto::{SigningKey, SshCheck, check_ssh_signature, issue};
use skald_github_stand_in::{App, GitHub, Request, User};
use skald_proto::{
    AccountId, Cbor as _, DeviceId, LATEST_HEADER, PROOF_HEADER, Role, SSH_LABEL, SSH_NAMESPACE,
    VERSION_HEADER, signed_data,
};
use ssh_key::encoding::Encode as _;
use ssh_key::private::Ed25519Keypair;
use ssh_key::{HashAlg, PrivateKey};

use fixtures::{Issuer, Keyed, add_device, hex};
use gate::Gate;
use machine::{DEVICES, Finished, Machine, NOT_LINKED, Run};
use world::{ACME, MEMBERS, Person, World, create, next_by, plain_lines, store, warnings, words};

/// The hidden setting of the GitHub API's address.
pub const GITHUB_API: &str = "SKALD_GITHUB_API_URL";

/// The GitHub users Ada and Bob sign in as, and their logins at sign-in.
pub const ADA_GITHUB: u64 = 7_340_215;
pub const BOB_GITHUB: u64 = 7_340_216;
const ADA_SIGN_IN: &str = "ada";
const BOB_SIGN_IN: &str = "bob";

/// The login GitHub's API gives Ada's GitHub user, and Bob's.
pub const LOGIN: &str = "ada-keys-q7w3";
pub const BOB_LOGIN: &str = "bob-keys-z9k2";

/// The host names of Ada's and Bob's machines, their certificates' `name`.
pub const ADA_HOST: &str = "ada-workstation";
pub const BOB_HOST: &str = "bob-workstation";

/// The `name` of the certificate of Ada's laptop.
pub const LAPTOP: &str = "ada-laptop";

/// An agent of Ada's machine.
pub const AGENT_NAME: &str = "claude";

/// A GitHub stand-in that plays GitHub's API, behind a gate.
pub struct Api {
    pub github: GitHub,
    gate: Gate,
}

impl Api {
    pub fn start() -> Self {
        let github = start_github();
        let gate = Gate::start(github.url().trim_start_matches("http://"));
        Api { github, gate }
    }

    /// Its address, the gate's, for [`GITHUB_API`].
    pub fn url(&self) -> String {
        self.gate.url()
    }

    /// The settings of a run that reaches GitHub's API here.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![(GITHUB_API.to_owned(), self.url())]
    }

    /// From now on, connections to it are dropped unanswered.
    pub fn unreachable(&self) {
        self.gate.close();
    }

    /// From now on, it answers again.
    pub fn reachable(&self) {
        self.gate.open();
    }
}

/// A GitHub stand-in with no users, for GitHub's API.
pub fn start_github() -> GitHub {
    GitHub::start(App {
        client_id: "Ov23li0standin0api".to_owned(),
        client_secret: "standin-secret-api-5e6f7a8b9c0d1e2f3a4b".to_owned(),
        callback: None,
    })
}

/// Makes `github` give the user `id` with `login`, and with the SSH keys `authentication` and
/// `signing`, OpenSSH public key lines.
pub fn list_user(github: &GitHub, id: u64, login: &str, authentication: &[&str], signing: &[&str]) {
    github.put_user(User::new(id, login));
    let keys = json!({ "user": id, "authentication": authentication, "signing": signing });
    control(github, "ssh-keys", &keys);
}

/// Where GitHub's API lists the key that signed the laptop's certificate among Ada's GitHub user's
/// keys, beside another key of each kind.
#[derive(Debug, Clone, Copy)]
pub enum Listed {
    Authentication,
    Signing,
    Nowhere,
}

/// Ada and Bob, GitHub's API at a stand-in of its own that gives Ada's GitHub user with [`LOGIN`]
/// and the laptop's key where `listed` says.
pub fn pair(listed: Listed) -> (Api, Scene) {
    let api = Api::start();
    let scene = Scene::pair(api.env());
    scene.list_ada(&api.github, LOGIN, listed);
    (api, scene)
}

/// Makes `github` play `faults`, as its control endpoint takes them, from now on.
pub fn set_faults(github: &GitHub, faults: &Value) {
    control(github, "faults", faults);
}

/// Puts `body` to `github`'s control endpoint `/_stand-in/<name>`; fails unless it takes it.
fn control(github: &GitHub, name: &str, body: &Value) {
    let url = format!("{}/_stand-in/{name}", github.url());
    let mut response = cloud::agent()
        .put(&url)
        .header("content-type", "application/json")
        .send(body.to_string().as_bytes())
        .unwrap_or_else(|err| panic!("PUT {url}: {err}"));
    let status = response.status().as_u16();
    let answer = response.body_mut().read_to_string().unwrap_or_default();
    assert_eq!(status, 204, "PUT {url} {body}: {answer}");
}

/// An Ed25519 SSH key of the tests' own.
pub fn ssh_key() -> PrivateKey {
    PrivateKey::from(Ed25519Keypair::from_seed(
        &SigningKey::generate().to_bytes(),
    ))
}

/// `key`'s public key as an OpenSSH public key line, as GitHub's API lists it.
pub fn line(key: &PrivateKey) -> String {
    key.public_key()
        .to_openssh()
        .expect("an OpenSSH public key line")
}

/// The binary form of an SSH signature by `key` of the device `device` of `account` with the
/// signing key `signing`, as the spec makes it.
pub fn ssh_signature(
    key: &PrivateKey,
    account: AccountId,
    device: DeviceId,
    signing: &[u8; 32],
) -> Vec<u8> {
    let data = signed_data(SSH_LABEL, &[account.as_bytes(), device.as_bytes(), signing]);
    let sig = key
        .sign(SSH_NAMESPACE, HashAlg::Sha512, &data)
        .expect("an SSH signature");
    sig.encode_vec().expect("the signature's binary form")
}

/// Runs `skald <args>` on `machine` with the settings `env` beside its own.
pub fn run_with(machine: &Machine, env: &[(String, String)], args: &[&str]) -> Finished {
    let mut command = machine.command_in(&machine.root().join("home"), args);
    command
        .stdin(Stdio::null())
        .envs(env.iter().map(|(name, value)| (name, value)));
    Run::spawn(&mut command).finish()
}

/// Ada's account, with her machine and her laptop, whose certificate carries an SSH signature by
/// [`Scene::key`] and names Ada's GitHub user; in most tests also Bob's machine, which created
/// `acme` and added Ada to it. No machine has run since the laptop was written.
pub struct Scene {
    pub world: World,
    pub ada: Person,
    pub bob: Option<Person>,
    pub laptop: Keyed,
    /// The key that signed the laptop's certificate, as an OpenSSH public key line.
    pub key: String,
    /// The settings of the person's runs: the GitHub API's address and any other.
    pub env: Vec<(String, String)>,
}

impl Scene {
    /// Ada and Bob, the person's runs with the settings `env`.
    pub fn pair(env: Vec<(String, String)>) -> Self {
        Self::new(true, env)
    }

    /// Ada alone, the person's runs with the settings `env`.
    pub fn alone(env: Vec<(String, String)>) -> Self {
        Self::new(false, env)
    }

    fn new(with_bob: bool, env: Vec<(String, String)>) -> Self {
        let world = World::start();
        let ada = world.sign_up(ADA_GITHUB, ADA_SIGN_IN, ADA_HOST);
        let bob = with_bob.then(|| {
            let bob = world.sign_up(BOB_GITHUB, BOB_SIGN_IN, BOB_HOST);
            let second = create(&world.cloud, &bob, ACME);
            let third = next_by(&second, &bob, |members| {
                members.push(ada.member(Role::Member));
            });
            store(&world.cloud, &bob.token, &third);
            bob
        });
        let key = ssh_key();
        let laptop = add_device(&world.cloud, ada.account, LAPTOP, &Issuer::Device(&ada));
        let cert = (world.cloud.cert(&ada.token, laptop.id)).expect("the laptop's certificate");
        let mut body = cert.body().clone();
        body.ssh = Some(ssh_signature(&key, ada.account, laptop.id, &body.signing));
        body.github = Some(ADA_GITHUB);
        assert_eq!(
            check_ssh_signature(&body, &[&line(&key)]),
            SshCheck::Passed { key: 0 },
            "the tests' SSH signature"
        );
        let signed = issue(&body, &ada.key, &laptop.signing).expect("a certificate");
        world.cloud.execute(&format!(
            "UPDATE devices SET cert = '\\x{}'::bytea WHERE id = '{}'::uuid",
            hex(&signed.to_cbor()),
            laptop.id
        ));
        Scene {
            world,
            ada,
            bob,
            laptop,
            key: line(&key),
            env,
        }
    }

    /// Makes `github` give Ada's GitHub user with `login` and the laptop's key where `listed`
    /// says.
    pub fn list_ada(&self, github: &GitHub, login: &str, listed: Listed) {
        let others = [line(&ssh_key()), line(&ssh_key())];
        let mut authentication = vec![others[0].as_str()];
        let mut signing = vec![others[1].as_str()];
        match listed {
            Listed::Authentication => authentication.push(&self.key),
            Listed::Signing => signing.push(&self.key),
            Listed::Nowhere => {}
        }
        list_user(github, ADA_GITHUB, login, &authentication, &signing);
    }

    /// Bob; fails in a scene without him.
    pub fn bob(&self) -> &Person {
        self.bob.as_ref().expect("a scene with Bob")
    }

    /// Runs `skald devices` on Ada's machine as the person; fails unless it succeeds.
    pub fn own(&self) -> Finished {
        let shown = run_with(&self.ada.machine, &self.env, &[DEVICES]);
        assert!(shown.success, "{shown:?}");
        shown
    }

    /// Runs `skald members acme` on Bob's machine as the person; fails unless it succeeds.
    pub fn member(&self) -> Finished {
        let shown = run_with(&self.bob().machine, &self.env, &[MEMBERS, ACME]);
        assert!(shown.success, "{shown:?}");
        shown
    }

    /// Runs [`Scene::own`] and, with Bob, [`Scene::member`].
    pub fn look(&self) -> Looked {
        Looked {
            own: self.own(),
            member: self.bob.is_some().then(|| self.member()),
        }
    }

    /// Fails unless both machines trust the laptop in `looked`.
    #[track_caller]
    pub fn assert_trusted(&self, looked: &Looked) {
        assert!(
            linked_in_devices(&looked.own, self.laptop.id),
            "Ada's machine does not trust the laptop: {looked:?}"
        );
        if let Some(member) = &looked.member {
            assert!(
                shown_in_members(member, self.laptop.id),
                "Bob's machine does not trust the laptop: {looked:?}"
            );
        }
    }

    /// Fails unless both machines trust the laptop and show it with `login` in `looked`, and
    /// neither warns of it.
    #[track_caller]
    pub fn assert_passed(&self, looked: &Looked, login: &str) {
        self.assert_trusted(looked);
        for shown in looked.outputs() {
            assert!(
                shows_login(shown, self.laptop.id, login),
                "the laptop is not shown with the login {login}: {looked:?}"
            );
            assert_eq!(
                warnings_naming(shown, self.laptop.id),
                Vec::<String>::new(),
                "a warning of the laptop: {looked:?}"
            );
        }
    }

    /// Fails unless both machines trust the laptop in `looked`, neither warns of it and no line
    /// but a warning holds `login`: the laptop is not checked.
    #[track_caller]
    pub fn assert_not_checked(&self, looked: &Looked, login: &str) {
        self.assert_trusted(looked);
        for shown in looked.outputs() {
            assert_eq!(
                warnings_naming(shown, self.laptop.id),
                Vec::<String>::new(),
                "a warning of the laptop: {looked:?}"
            );
            assert!(
                !(plain_lines(shown).iter()).any(|line| words(line).contains(&login)),
                "{login} is shown: {looked:?}"
            );
        }
    }
}

/// What the machines showed in one look.
#[derive(Debug)]
pub struct Looked {
    /// `skald devices` on Ada's machine.
    pub own: Finished,
    /// `skald members acme` on Bob's machine, in a scene with Bob.
    pub member: Option<Finished>,
}

impl Looked {
    /// Each machine's output.
    pub fn outputs(&self) -> Vec<&Finished> {
        std::iter::once(&self.own).chain(&self.member).collect()
    }
}

/// The lines of `shown` that are no warnings and hold `device`'s ID.
pub fn device_lines(shown: &Finished, device: DeviceId) -> Vec<String> {
    let id = device.to_string();
    let mut lines = plain_lines(shown);
    lines.retain(|line| line.contains(&id));
    lines
}

/// Whether `shown`, the output of `skald devices`, lists `device` as linked: on a line that is no
/// warning, above the line that says `not linked`, with the laptop's name.
pub fn linked_in_devices(shown: &Finished, device: DeviceId) -> bool {
    let id = device.to_string();
    let above: Vec<&str> = (shown.stdout.lines())
        .take_while(|line| !line.to_lowercase().contains(NOT_LINKED))
        .filter(|line| !line.to_lowercase().contains(world::WARNING))
        .collect();
    above
        .iter()
        .any(|line| line.contains(&id) && words(line).contains(&LAPTOP))
}

/// Whether `shown`, the output of `skald members`, shows `device` as a device with a valid
/// certificate: on a line that is no warning, holding its ID and the laptop's name.
pub fn shown_in_members(shown: &Finished, device: DeviceId) -> bool {
    (device_lines(shown, device).iter()).any(|line| words(line).contains(&LAPTOP))
}

/// Whether `shown` shows `device` with the GitHub login `login`: a line that is no warning holds
/// its ID and `login` as a word.
pub fn shows_login(shown: &Finished, device: DeviceId, login: &str) -> bool {
    (device_lines(shown, device).iter()).any(|line| words(line).contains(&login))
}

/// The warning lines of `shown` that hold `device`'s ID.
pub fn warnings_naming(shown: &Finished, device: DeviceId) -> Vec<String> {
    let id = device.to_string().to_lowercase();
    let mut lines = warnings(shown);
    lines.retain(|line| line.contains(&id));
    lines
}

/// Fails unless `requests` came without credentials and carry nothing of `person`'s session: no
/// `authorization` or `cookie` header, none of the headers the client sends its server, and no
/// header or parameter holds the session token.
#[track_caller]
pub fn assert_carry_nothing_of_the_session(requests: &[Request], person: &Person) {
    let banned = [
        "authorization",
        "cookie",
        VERSION_HEADER,
        PROOF_HEADER,
        LATEST_HEADER,
    ];
    for request in requests {
        for name in banned {
            assert!(
                !request.headers.contains_key(&name.to_lowercase()),
                "a request to GitHub with {name}: {request:?}"
            );
        }
        let values = (request.headers.values()).chain(request.params.values());
        for value in values {
            assert!(
                !value.contains(&person.token),
                "a request to GitHub with the session token: {request:?}"
            );
        }
    }
}
