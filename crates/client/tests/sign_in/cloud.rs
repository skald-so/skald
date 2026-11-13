//! A `skald-server` with GitHub sign-in through the GitHub stand-in, on a fresh PostgreSQL
//! database, behind a recording relay; and the person who approves or denies a machine's code on
//! its verification page in headless Chrome.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_github_stand_in::{App, GitHub, User};
use skald_proto::{AccountId, Cbor, Cert, DeviceId, Roster, SpaceId};
use skald_server::{CALLBACK_PATH, ClientSecret, GitHubApp, Settings, SignInSettings};

use super::browser::{APPROVE, CODE_FIELD, Page, SIGN_IN_LINK, SUBMIT, Visit};
use super::common::Server;
use super::machine::{self, Finished, Machine, Run};
use super::relay::{Relay, strings_named};

/// How long a test waits for the server to see what it expects.
pub const WAIT: Duration = Duration::from_secs(60);

/// The fake OAuth App's client ID and secret.
const CLIENT_ID: &str = "Ov23li0standin0cli";
const CLIENT_SECRET: &str = "standin-secret-cli-0b1c2d3e4f5a6b7c8d9e";

/// The requests that register a device and give and ask for the account's master key, as their
/// lines start.
pub const REGISTER: &str = "POST /devices ";
pub const GIVE_MASTER: &str = "PUT /account/master ";
pub const ASK_MASTER: &str = "GET /account/master ";

/// What device authorization gave the machine, as the relay saw it.
#[derive(Clone, Debug)]
pub struct Grant {
    pub user_code: String,
    pub verification_uri: String,
}

/// A device of the account, as `GET /devices` gives it.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub id: DeviceId,
    pub cert: Option<Cert>,
}

/// The IDs of `devices`.
pub fn ids(devices: &[Device]) -> Vec<DeviceId> {
    devices.iter().map(|device| device.id).collect()
}

/// The devices of `devices` that have a certificate.
pub fn certified(devices: &[Device]) -> Vec<DeviceId> {
    devices
        .iter()
        .filter(|device| device.cert.is_some())
        .map(|device| device.id)
        .collect()
}

fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(text)
        .unwrap_or_else(|err| panic!("{text:?} is not base64url without padding: {err}"))
}

pub struct Cloud {
    /// Where machines reach the server: the relay.
    pub url: String,
    pub relay: Relay,
    pub github: GitHub,
    server: Server,
}

impl Cloud {
    /// A server whose codes work for `lifetime` seconds and poll every `interval`.
    pub fn start(lifetime: u64, interval: u64) -> Self {
        let mut github = None;
        let server = Server::start_with(|addr| {
            // Chrome resolves every *.test host to the server.
            let public = format!("http://skald.test:{}", addr.port());
            let stand_in = GitHub::start(App {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                callback: Some(format!("{public}{CALLBACK_PATH}")),
            });
            let github_url = stand_in.url().parse().expect("the stand-in's address");
            let sign_in = SignInSettings {
                public_url: public.parse().expect("a public address"),
                github: Some(GitHubApp {
                    client_id: CLIENT_ID.to_owned(),
                    client_secret: ClientSecret::new(CLIENT_SECRET.to_owned())
                        .expect("a client secret"),
                    api_url: github_url,
                    url: stand_in.url().parse().expect("the stand-in's address"),
                }),
                google: None,
                state_lifetime: Duration::from_secs(600),
                device_code_lifetime: Duration::from_secs(lifetime),
                device_poll_interval: Duration::from_secs(interval),
            };
            github = Some(stand_in);
            Settings {
                sign_in: Some(sign_in),
                ..Settings::default()
            }
        });
        let relay = Relay::start(server.url.trim_start_matches("http://"));
        Cloud {
            url: relay.url.clone(),
            relay,
            github: github.expect("the stand-in started with the server"),
            server,
        }
    }

    /// Adds a GitHub user to the stand-in and gives it.
    pub fn user(&self, id: u64, login: &str) -> User {
        self.github.put_user(User::new(id, login));
        User::new(id, login)
    }

    /// Every grant device authorization gave so far, in order.
    pub fn grants(&self) -> Vec<Grant> {
        let answered = self.relay.answered();
        let codes = strings_named(&answered, "user_code");
        let pages = strings_named(&answered, "verification_uri");
        codes
            .into_iter()
            .zip(pages)
            .map(|(user_code, verification_uri)| Grant {
                user_code,
                verification_uri,
            })
            .collect()
    }

    /// The last grant device authorization gave; fails if it gave none.
    pub fn last_grant(&self) -> Grant {
        let grants = self.grants();
        grants
            .last()
            .cloned()
            .expect("device authorization gave a code")
    }

    /// Every session token the server gave so far, in order.
    pub fn tokens(&self) -> Vec<String> {
        strings_named(&self.relay.answered(), "access_token")
    }

    /// The session token the server gave last.
    pub fn last_token(&self) -> String {
        self.tokens().last().cloned().expect("a session token")
    }

    /// The files of `machine` that hold a session token the server gave, each with that token.
    pub fn token_files(&self, machine: &Machine) -> Vec<(PathBuf, String)> {
        let mut found = Vec::new();
        for token in self.tokens() {
            for file in machine.files_holding(&token) {
                found.push((file, token.clone()));
            }
        }
        found
    }

    /// The `GET /session` answer to a request with the session token `token`, straight to the
    /// server: the status and the body.
    pub fn session_of(&self, token: &str) -> (u16, Value) {
        self.get_as("/session", token)
    }

    /// The answer to `GET <path>` with the session token `token`, straight to the server: the
    /// status and the body.
    pub fn get_as(&self, path: &str, token: &str) -> (u16, Value) {
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent
            .get(format!("{}{path}", self.server.url))
            .header(skald_proto::VERSION_HEADER, version)
            .header("authorization", format!("Bearer {token}"))
            .call()
            .unwrap_or_else(|err| panic!("GET {path}: {err}"));
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .unwrap_or_else(|err| panic!("read GET {path}: {err}"));
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
    }

    /// The account's master public key, from the server, for a session with `token`.
    pub fn master(&self, token: &str) -> Option<[u8; 32]> {
        let (status, body) = self.get_as("/account/master", token);
        assert_eq!(status, 200, "GET /account/master: {body}");
        match &body["master"] {
            Value::Null => None,
            Value::String(text) => Some(unb64(text).try_into().expect("a 32-byte master key")),
            other => panic!("GET /account/master: {other}"),
        }
    }

    /// The account's devices, from the server, for a session with `token`, each with its
    /// certificate as the device sent it.
    pub fn devices(&self, token: &str) -> Vec<Device> {
        let (status, body) = self.get_as("/devices", token);
        assert_eq!(status, 200, "GET /devices: {body}");
        body["devices"]
            .as_array()
            .unwrap_or_else(|| panic!("GET /devices: {body}"))
            .iter()
            .map(|device| Device {
                id: device["device"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .unwrap_or_else(|| panic!("a device ID: {device}")),
                cert: device["cert"].as_str().map(|cert| {
                    Cert::from_cbor(&unb64(cert))
                        .unwrap_or_else(|err| panic!("a certificate: {err}"))
                }),
            })
            .collect()
    }

    /// The revisions of `space` the server holds, for the session `token`, straight from the
    /// server: each one's bytes as stored, and the revision they decode to.
    pub fn revisions(&self, token: &str, space: SpaceId) -> Vec<(Vec<u8>, Roster)> {
        let path = format!("/spaces/{space}/revisions");
        let (status, body) = self.get_as(&path, token);
        assert_eq!(status, 200, "GET {path}: {body}");
        body["revisions"]
            .as_array()
            .unwrap_or_else(|| panic!("GET {path}: {body}"))
            .iter()
            .map(|revision| {
                let bytes = unb64(revision.as_str().expect("a revision in base64url"));
                let roster = Roster::from_cbor(&bytes)
                    .unwrap_or_else(|err| panic!("a revision that does not decode: {err}"));
                (bytes, roster)
            })
            .collect()
    }

    /// How many requests whose line starts with `request` the machines sent from `since` on.
    pub fn count_sent(&self, since: Instant, request: &str) -> usize {
        let request = request.to_lowercase();
        self.relay
            .sent_since(since)
            .iter()
            .filter(|sent| sent.head.starts_with(&request))
            .count()
    }

    /// Starts `skald sign-in --server <this server> <extra>` on `machine`, approves the code it
    /// shows as `user` at `page`, and waits for the run to finish.
    pub fn signed_in(
        &self,
        page: &Page,
        machine: &Machine,
        user: &User,
        extra: &[&str],
    ) -> Finished {
        let before = self.grants().len();
        let mut args = vec![machine::SIGN_IN, machine::SERVER, &self.url];
        args.extend_from_slice(extra);
        let mut run = machine.start(&args);
        let grant = shown_grant(self, &mut run, before);
        self.decide(page, user, &grant, APPROVE);
        run.finish()
    }

    /// A new machine with the host name `host`, signed in as the first device of `user`'s account
    /// as [`Cloud::signed_in`] signs it in; gives it and what its sign-in printed.
    pub fn first_machine(&self, page: &Page, user: &User, host: &str) -> (Machine, Finished) {
        let machine = Machine::new().with_host_name(host);
        let first = [machine::FIRST_DEVICE, machine::YES];
        let finished = self.signed_in(page, &machine, user, &first);
        (machine, finished)
    }

    /// Waits until the relay has lost an answer of the server to `request`, for up to [`WAIT`],
    /// then kills `run`, which sent it.
    pub fn kill_once_lost(&self, run: Run, request: &str) {
        let deadline = Instant::now() + WAIT;
        while self.relay.lost() == 0 {
            assert!(Instant::now() < deadline, "no {request}: {}", run.shown());
            thread::sleep(Duration::from_millis(50));
        }
        drop(run);
    }

    /// What every table of the server's database holds, one XML document per table.
    pub fn rows(&self) -> Vec<String> {
        self.server.rows()
    }

    /// Runs `sql` on the server's database.
    pub fn execute(&self, sql: &str) {
        self.server.execute(sql);
    }

    /// The account a request with `token` acts as, from the server; fails if it acts as none.
    pub fn account_of(&self, token: &str) -> AccountId {
        let (status, body) = self.session_of(token);
        assert_eq!(status, 200, "GET /session: {body}");
        body["account"]
            .as_str()
            .and_then(|id| id.parse().ok())
            .unwrap_or_else(|| panic!("an account ID: {body}"))
    }

    /// The GitHub login of the account a request with `token` acts as; fails if it acts as none.
    pub fn login_of(&self, token: &str) -> String {
        let (status, body) = self.session_of(token);
        assert_eq!(status, 200, "GET /session: {body}");
        body["github_login"]
            .as_str()
            .expect("a GitHub login")
            .to_owned()
    }

    /// The person at `page`, signed in to GitHub as `user`, opens `grant`'s verification page,
    /// signing in to the server on the way, types its code and clicks `button`, [`APPROVE`] or
    /// [`DENY`](super::browser::DENY); gives the last visit.
    pub fn decide(&self, page: &Page, user: &User, grant: &Grant, button: &str) -> Visit {
        let visit = page.goto(&self.github.sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        page.goto_settled(&grant.verification_uri);
        if page.count(SIGN_IN_LINK) > 0 {
            page.click(&format!("{SIGN_IN_LINK} >> nth=0"));
            page.goto_settled(&grant.verification_uri);
        }
        assert_eq!(
            page.count(CODE_FIELD),
            1,
            "a field for the code: {}",
            page.text()
        );
        page.fill(CODE_FIELD, &grant.user_code);
        if page.count(button) == 0 {
            page.click(SUBMIT);
        }
        let visit = page.click(button);
        assert!(
            visit
                .status
                .is_some_and(|status| (200..300).contains(&status)),
            "{button}: {visit:?}"
        );
        visit
    }
}

/// Waits until device authorization has given `run` its grant after the `before` it gave earlier
/// and `run` shows its verification page's address and code; gives the grant.
pub fn shown_grant(cloud: &Cloud, run: &mut Run, before: usize) -> Grant {
    let deadline = Instant::now() + WAIT;
    while cloud.grants().len() <= before {
        run.assert_running();
        assert!(
            Instant::now() < deadline,
            "no device authorization: {}",
            run.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
    let grant = cloud.last_grant();
    run.wait_to_show(&[&grant.verification_uri, &grant.user_code]);
    grant
}
