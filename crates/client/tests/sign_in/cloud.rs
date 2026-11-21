//! A `skald-server` with GitHub sign-in through the GitHub stand-in, Google sign-in through the
//! OIDC stand-in, which plays Google, or both, on a fresh PostgreSQL database, behind a recording
//! relay; and the person who approves or denies a machine's code on its verification page in
//! headless Chrome, signing in there with GitHub or Google.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_github_stand_in::{App, GitHub, User};
use skald_oidc_stand_in::{Client, Key, Person, Provider};
use skald_proto::{AccountId, Cbor, Cert, DeviceId, Roster, SpaceId};
use skald_server::{
    CALLBACK_PATH, ClientSecret, GitHubApp, OidcSettings, Settings, SignInSettings,
};

use super::browser::{APPROVE, CODE_FIELD, Page, SIGN_IN_GITHUB, SIGN_IN_GOOGLE, SUBMIT, Visit};
use super::common::Server;
use super::machine::{self, Finished, Machine, Run};
use super::relay::{Relay, count_heads, strings_named};

#[path = "../../../server/tests/common/agent.rs"]
mod agent;
pub use agent::{agent, get};
#[path = "recovery.rs"]
mod recovery;

/// How long a test waits for the server to see what it expects.
pub const WAIT: Duration = Duration::from_secs(60);

/// The fake OAuth App's client ID and secret.
const CLIENT_ID: &str = "Ov23li0standin0cli";
const CLIENT_SECRET: &str = "standin-secret-cli-0b1c2d3e4f5a6b7c8d9e";
/// The server's client at the OIDC stand-in, and Google's callback under the public address.
const GOOGLE_CLIENT_ID: &str = "skald-stand-in-google-cli";
const GOOGLE_CLIENT_SECRET: &str = "stand-in-google-secret-cli-4d2a9e7b1c3f";
const GOOGLE_CALLBACK: &str = "/auth/google/callback";

/// The providers the server has settings for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Providers {
    GitHub,
    Google,
    Both,
}

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

/// The bytes of `text`, base64url without padding.
pub fn unb64(text: &str) -> Vec<u8> {
    URL_SAFE_NO_PAD
        .decode(text)
        .unwrap_or_else(|err| panic!("{text:?} is not base64url without padding: {err}"))
}

/// What the clouds of a test target share and a test that times the client takes alone
/// ([`alone`]). Under `cargo test`, the tests of a target are threads of one process: no cloud of
/// another test then runs beside the timed one. Under nextest, each test is a process of its own.
static BESIDE: RwLock<()> = RwLock::new(());

thread_local! {
    /// What this test's thread holds of [`BESIDE`].
    static HELD: RefCell<Held> = const { RefCell::new(Held::Nothing) };
}

/// What a test's thread holds of [`BESIDE`]: nothing; all of it; or a share, for this many of its
/// clouds, taken once so that a test waiting to be alone holds back no cloud of its thread.
enum Held {
    Nothing,
    Alone(RwLockWriteGuard<'static, ()>),
    Shared(RwLockReadGuard<'static, ()>, usize),
}

/// Waits until no cloud of another test of the target runs, and keeps any from starting until
/// dropped; the test's own clouds start as before.
pub fn alone() -> Alone {
    let guard = BESIDE.write().unwrap_or_else(PoisonError::into_inner);
    HELD.with(|held| {
        let mut held = held.borrow_mut();
        assert!(
            matches!(*held, Held::Nothing),
            "a cloud runs, or the test is alone already"
        );
        *held = Held::Alone(guard);
    });
    Alone
}

/// A test alone among the target's clouds ([`alone`]) until dropped.
pub struct Alone;

impl Drop for Alone {
    fn drop(&mut self) {
        HELD.with(|held| *held.borrow_mut() = Held::Nothing);
    }
}

/// A cloud's share of [`BESIDE`], held by its thread until dropped.
struct Share;

impl Share {
    /// Waits while a test is alone, unless it is this thread's.
    fn take() -> Self {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            match &mut *held {
                Held::Nothing => {
                    let guard = BESIDE.read().unwrap_or_else(PoisonError::into_inner);
                    *held = Held::Shared(guard, 1);
                }
                Held::Shared(_, clouds) => *clouds += 1,
                Held::Alone(_) => {}
            }
        });
        Share
    }
}

impl Drop for Share {
    fn drop(&mut self) {
        HELD.with(|held| {
            let mut held = held.borrow_mut();
            if let Held::Shared(_, clouds) = &mut *held {
                *clouds -= 1;
                if *clouds == 0 {
                    *held = Held::Nothing;
                }
            }
        });
    }
}

pub struct Cloud {
    /// Where machines reach the server, and the tests' own requests too: the relay, or the plain
    /// HTTP front before it. Behind a TLS front, the relay; machines reach the server at
    /// [`Cloud::public`] there.
    pub url: String,
    pub relay: Relay,
    /// The GitHub stand-in; it runs also when the server has no GitHub settings.
    pub github: GitHub,
    /// The OIDC stand-in, when the server has Google's settings.
    pub google: Option<Provider>,
    /// The server's public address, e.g. `http://skald.test:41234`.
    pub public: String,
    server: Server,
    /// Dropped last, once the server has stopped.
    _share: Share,
}

impl Cloud {
    /// A server with GitHub's settings only, whose codes work for `lifetime` seconds and poll every
    /// `interval`.
    pub fn start(lifetime: u64, interval: u64) -> Self {
        Self::with(Providers::GitHub, lifetime, interval)
    }

    /// A server with the settings of `providers`, whose codes work for `lifetime` seconds and poll
    /// every `interval`.
    pub fn with(providers: Providers, lifetime: u64, interval: u64) -> Self {
        // Chrome resolves every *.test host to the server.
        Self::serving(providers, "skald.test", lifetime, interval)
    }

    /// As [`Cloud::start`], the server's public address `http://<host>:<port>`, such as
    /// `localhost` or `127.0.0.1`, which Chrome reaches.
    pub fn on_host(host: &str, lifetime: u64, interval: u64) -> Self {
        Self::serving(Providers::GitHub, host, lifetime, interval)
    }

    /// As [`Cloud::on_host`], the server's public address `http://<host>:<front>`: that of a front
    /// before the relay that the test runs on the port `front` and points at the relay once the
    /// cloud has started. Machines and the person's browser reach the server through the front, at
    /// [`Cloud::url`], so the origin a machine signs on connecting is the server's public address.
    pub fn fronted(host: &str, front: u16, lifetime: u64, interval: u64) -> Self {
        Self::fronted_with(host, front, lifetime, interval, |_| {})
    }

    /// As [`Cloud::fronted`], the server's settings changed by `change`.
    pub fn fronted_with(
        host: &str,
        front: u16,
        lifetime: u64,
        interval: u64,
        change: impl FnOnce(&mut Settings),
    ) -> Self {
        Self::serving_at(
            Providers::GitHub,
            &format!("http://{host}"),
            Some(front),
            (lifetime, interval),
            change,
        )
    }

    /// As [`Cloud::fronted`], the server's public address `https://<host>:<front>`: that of a TLS
    /// front, such as `https/front.rs`, which the test runs on the port `front`. Machines reach the
    /// server there, at [`Cloud::public`]; [`Cloud::url`] is the relay's.
    pub fn fronted_over_tls(host: &str, front: u16, lifetime: u64, interval: u64) -> Self {
        Self::fronted_over_tls_with(host, front, (lifetime, interval), |_| {})
    }

    /// As [`Cloud::fronted_over_tls`], codes living and polling as `(lifetime, interval)` says, and
    /// the server's settings changed by `change`.
    pub fn fronted_over_tls_with(
        host: &str,
        front: u16,
        codes: (u64, u64),
        change: impl FnOnce(&mut Settings),
    ) -> Self {
        Self::serving_at(
            Providers::GitHub,
            &format!("https://{host}"),
            Some(front),
            codes,
            change,
        )
    }

    /// As [`Cloud::with`], the server's public address `http://<host>:<port>`.
    fn serving(providers: Providers, host: &str, lifetime: u64, interval: u64) -> Self {
        let site = format!("http://{host}");
        Self::serving_at(providers, &site, None, (lifetime, interval), |_| {})
    }

    /// As [`Cloud::serving`], the public address `<site>:<port>`, `site` its scheme and host such
    /// as `http://skald.test`, its port `front` if given, machines reaching the server there,
    /// otherwise the server's own, machines reaching the server at the relay; codes living and
    /// polling as `(lifetime, interval)` says, and the settings changed by `change`.
    fn serving_at(
        providers: Providers,
        site: &str,
        front: Option<u16>,
        (lifetime, interval): (u64, u64),
        change: impl FnOnce(&mut Settings),
    ) -> Self {
        let share = Share::take();
        let google = (providers != Providers::GitHub).then(|| Provider::start(&[Key::rsa()]));
        let mut github = None;
        let mut public_url = String::new();
        let server = Server::start_with(|addr| {
            let public = format!("{site}:{}", front.unwrap_or(addr.port()));
            let stand_in = GitHub::start(App {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                callback: Some(format!("{public}{CALLBACK_PATH}")),
            });
            let github_url = stand_in.url().parse().expect("the stand-in's address");
            let oidc = google.as_ref().map(|provider| {
                provider.register(Client {
                    client_id: GOOGLE_CLIENT_ID.to_owned(),
                    client_secret: GOOGLE_CLIENT_SECRET.to_owned(),
                    redirect_uri: format!("{public}{GOOGLE_CALLBACK}"),
                });
                OidcSettings {
                    issuer: provider.issuer().to_owned(),
                    client_id: GOOGLE_CLIENT_ID.to_owned(),
                    client_secret: ClientSecret::new(GOOGLE_CLIENT_SECRET.to_owned())
                        .expect("a client secret"),
                    key_refresh: Duration::from_secs(60),
                }
            });
            let sign_in = SignInSettings {
                public_url: public.parse().expect("a public address"),
                web_url: None,
                github: (providers != Providers::Google).then(|| GitHubApp {
                    client_id: CLIENT_ID.to_owned(),
                    client_secret: ClientSecret::new(CLIENT_SECRET.to_owned())
                        .expect("a client secret"),
                    api_url: github_url,
                    url: stand_in.url().parse().expect("the stand-in's address"),
                }),
                google: oidc,
                state_lifetime: Duration::from_secs(600),
                link_age: Duration::from_secs(300),
                device_code_lifetime: Duration::from_secs(lifetime),
                device_poll_interval: Duration::from_secs(interval),
            };
            github = Some(stand_in);
            public_url = public;
            let mut settings = Settings {
                sign_in: Some(sign_in),
                ..Settings::default()
            };
            change(&mut settings);
            settings
        });
        let relay = Relay::start(server.url.trim_start_matches("http://"));
        Cloud {
            url: match front {
                Some(_) if site.starts_with("http://") => public_url.clone(),
                _ => relay.url.clone(),
            },
            relay,
            github: github.expect("the stand-in started with the server"),
            google,
            public: public_url,
            server,
            _share: share,
        }
    }

    /// The OIDC stand-in; fails unless the server has Google's settings.
    pub fn provider(&self) -> &Provider {
        self.google
            .as_ref()
            .expect("the server has Google's settings")
    }

    /// Adds a person to the OIDC stand-in and gives them.
    pub fn person(&self, subject: &str, email: &str) -> Person {
        let person = Person::new(subject, email);
        self.provider().put_person(person.clone());
        person
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

    /// Waits until device authorization has given a grant after the `before` it gave earlier, for
    /// up to [`WAIT`], and gives it; meanwhile `running` fails if the run that asked for it ended,
    /// and gives what that run printed.
    #[track_caller]
    pub fn next_grant(&self, before: usize, mut running: impl FnMut() -> String) -> Grant {
        let deadline = Instant::now() + WAIT;
        while self.grants().len() <= before {
            let shown = running();
            assert!(
                Instant::now() < deadline,
                "no device authorization: {shown}"
            );
            thread::sleep(Duration::from_millis(50));
        }
        self.last_grant()
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
        let mut response = get(|| {
            agent()
                .get(format!("{}{path}", self.server.url))
                .header(skald_proto::VERSION_HEADER, &version)
                .header("authorization", format!("Bearer {token}"))
                .call()
        })
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

    /// The certificate the server holds of `device`, for a session with `token`, if any.
    pub fn cert(&self, token: &str, device: DeviceId) -> Option<Cert> {
        (self.devices(token).into_iter())
            .find(|held| held.id == device)
            .and_then(|held| held.cert)
    }

    /// The one device of the account that is not among `known`, for a session with `token`.
    #[track_caller]
    pub fn new_device(&self, token: &str, known: &[DeviceId]) -> DeviceId {
        let mut new = ids(&self.devices(token));
        new.retain(|device| !known.contains(device));
        assert_eq!(new.len(), 1, "not one new device: {new:?}");
        new[0]
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

    /// The spaces the account of the session `token` gets, straight from the server, in the order
    /// it gives them: each one's ID and name.
    pub fn spaces(&self, token: &str) -> Vec<(SpaceId, String)> {
        let (status, body) = self.get_as("/spaces", token);
        assert_eq!(status, 200, "GET /spaces: {body}");
        body["spaces"]
            .as_array()
            .unwrap_or_else(|| panic!("GET /spaces: {body}"))
            .iter()
            .map(|space| {
                let id = space["space"]
                    .as_str()
                    .and_then(|id| id.parse().ok())
                    .unwrap_or_else(|| panic!("a space ID: {space}"));
                let name = space["name"]
                    .as_str()
                    .unwrap_or_else(|| panic!("a space name: {space}"));
                (id, name.to_owned())
            })
            .collect()
    }

    /// How many requests whose line starts with `request` the machines sent from `since` on.
    pub fn count_sent(&self, since: Instant, request: &str) -> usize {
        count_heads(&self.relay.sent_since(since), request)
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
        self.approved(machine, extra, |grant| {
            self.decide(page, user, grant, APPROVE);
        })
    }

    /// As [`Cloud::signed_in`], the person approving as `person`, signed in with Google.
    pub fn signed_in_with_google(
        &self,
        page: &Page,
        machine: &Machine,
        person: &Person,
        extra: &[&str],
    ) -> Finished {
        self.approved(machine, extra, |grant| {
            self.decide_with_google(page, person, grant, APPROVE);
        })
    }

    /// Starts `skald sign-in --server <this server> <extra>` on `machine`, runs `approve` on the
    /// grant it shows, and waits for the run to finish; then shows the recovery key it leaves
    /// waiting, as the person does at a terminal.
    fn approved(
        &self,
        machine: &Machine,
        extra: &[&str],
        approve: impl FnOnce(&Grant),
    ) -> Finished {
        let before = self.grants().len();
        let mut args = vec![machine::SIGN_IN, machine::SERVER, &self.url];
        args.extend_from_slice(extra);
        let mut run = machine.start(&args);
        let grant = shown_grant(self, &mut run, before);
        approve(&grant);
        let finished = run.finish();
        recovery::shown_if_waiting(machine, &self.url, &finished);
        finished
    }

    /// A new machine with the host name `host`, signed in as the first device of `user`'s account
    /// as [`Cloud::signed_in`] signs it in; gives it and what its sign-in printed.
    pub fn first_machine(&self, page: &Page, user: &User, host: &str) -> (Machine, Finished) {
        let machine = Machine::new().with_host_name(host);
        let finished = self.signed_in(page, &machine, user, &[]);
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
    /// signing in to the server with GitHub on the way, types its code and clicks `button`,
    /// [`APPROVE`] or [`DENY`](super::browser::DENY); gives the last visit.
    pub fn decide(&self, page: &Page, user: &User, grant: &Grant, button: &str) -> Visit {
        let visit = page.goto(&self.github.sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        self.decide_signing_in(page, SIGN_IN_GITHUB, grant, button)
    }

    /// As [`Cloud::decide`], the person signed in to Google as `person` and signing in to the
    /// server with Google on the verification page.
    pub fn decide_with_google(
        &self,
        page: &Page,
        person: &Person,
        grant: &Grant,
        button: &str,
    ) -> Visit {
        let visit = page.goto(&self.provider().sign_in_url(&person.subject));
        assert_eq!(visit.status, Some(200), "OIDC stand-in sign-in: {visit:?}");
        self.decide_signing_in(page, SIGN_IN_GOOGLE, grant, button)
    }

    /// The person at `page` opens `grant`'s verification page, signing in to the server on the way
    /// by the sign-in that `link` finds there, types its code and clicks `button`; gives the last
    /// visit.
    fn decide_signing_in(&self, page: &Page, link: &str, grant: &Grant, button: &str) -> Visit {
        loaded(page.goto_settled(&grant.verification_uri));
        if page.count(CODE_FIELD) == 0 {
            assert_eq!(
                page.count(link),
                1,
                "the page offers {link}: {}",
                page.text()
            );
            loaded(page.click_settled(&format!("{link} >> nth=0")));
            loaded(page.goto_settled(&grant.verification_uri));
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

/// Fails unless `visit`'s page loaded: a page that failed to load offers nothing.
fn loaded(visit: Visit) {
    assert!(
        visit.error.is_none() && visit.status == Some(200),
        "the page did not load: {visit:?}"
    );
}

/// Waits until device authorization has given `run` its grant after the `before` it gave earlier
/// and `run` shows its verification page's address and code; gives the grant.
pub fn shown_grant(cloud: &Cloud, run: &mut Run, before: usize) -> Grant {
    let grant = cloud.next_grant(before, || {
        run.assert_running();
        run.shown()
    });
    run.wait_to_show(&[&grant.verification_uri, &grant.user_code]);
    grant
}
