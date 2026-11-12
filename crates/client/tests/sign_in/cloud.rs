//! A `skald-server` with GitHub sign-in through the GitHub stand-in, on a fresh PostgreSQL
//! database, behind a recording relay; and the person who approves or denies a machine's code on
//! its verification page in headless Chrome.

use std::time::Duration;

use serde_json::Value;
use skald_github_stand_in::{App, GitHub, User};
use skald_server::{CALLBACK_PATH, ClientSecret, GitHubApp, Settings, SignInSettings};

use super::browser::{Page, Visit};
use super::common::Server;
use super::relay::{Relay, strings_named};

/// The fake OAuth App's client ID and secret.
const CLIENT_ID: &str = "Ov23li0standin0cli";
const CLIENT_SECRET: &str = "standin-secret-cli-0b1c2d3e4f5a6b7c8d9e";

/// The verification page's field for the code, the buttons that approve and deny it, and the
/// button of a page that asks for the code first.
const CODE_FIELD: &str = "role=textbox";
pub const APPROVE: &str = "role=button[name=/approve/i]";
pub const DENY: &str = "role=button[name=/deny/i]";
const SUBMIT: &str = "role=button >> nth=0";
/// A link or button of the verification page that signs a browser without a session in.
const SIGN_IN: &str = "a:has-text('sign in'), button:has-text('sign in')";

/// What device authorization gave the machine, as the relay saw it.
#[derive(Clone, Debug)]
pub struct Grant {
    pub user_code: String,
    pub verification_uri: String,
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
                github: GitHubApp {
                    client_id: CLIENT_ID.to_owned(),
                    client_secret: ClientSecret::new(CLIENT_SECRET.to_owned())
                        .expect("a client secret"),
                    api_url: github_url,
                    url: stand_in.url().parse().expect("the stand-in's address"),
                },
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

    /// The `GET /session` answer to a request with the session token `token`, straight to the
    /// server: the status and the body.
    pub fn session_of(&self, token: &str) -> (u16, Value) {
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .build()
            .new_agent();
        let mut response = agent
            .get(format!("{}/session", self.server.url))
            .header(skald_proto::VERSION_HEADER, version)
            .header("authorization", format!("Bearer {token}"))
            .call()
            .expect("GET /session");
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .read_to_string()
            .expect("read the session");
        (status, serde_json::from_str(&body).unwrap_or(Value::Null))
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
    /// [`DENY`]; gives the last visit.
    pub fn decide(&self, page: &Page, user: &User, grant: &Grant, button: &str) -> Visit {
        let visit = page.goto(&self.github.sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        page.goto_settled(&grant.verification_uri);
        if page.count(SIGN_IN) > 0 {
            page.click(&format!("{SIGN_IN} >> nth=0"));
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
