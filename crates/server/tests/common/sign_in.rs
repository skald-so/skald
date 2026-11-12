//! A `skald-server` set up for GitHub sign-in against the GitHub stand-in, and a person signing in
//! to it in a browser.

use std::time::Duration;

use serde_json::Value;
use skald_github_stand_in::{App, GitHub, User};

use super::browser::{Browser, Page, Visit};
use super::{Server, TestDb, on_free_port};

/// The fake OAuth App's client ID and secret.
pub const CLIENT_ID: &str = "Ov23li0standin0client";
pub const CLIENT_SECRET: &str = "standin-secret-5c0f1e7a9b2d4c6e8f0a1b3c5d7e9f1a";

/// Where the browser starts signing in: it takes the return address as `return_to`.
pub const START: &str = "/auth/github";
/// The GitHub callback, under the server's public address.
pub const CALLBACK: &str = "/auth/github/callback";
/// The session's account: `{"account": <ID>, "github_login": <login>}`, `401` without a session.
pub const SESSION: &str = "/session";

/// The server's settings that name the public address, the OAuth App and GitHub's addresses.
pub fn github_settings<'a>(public: &'a str, github: &'a GitHub) -> Vec<&'a str> {
    vec![
        "--public-url",
        public,
        "--github-client-id",
        CLIENT_ID,
        "--github-url",
        github.url(),
        "--github-api-url",
        github.url(),
    ]
}

/// A server with GitHub sign-in through the stand-in, on its own database.
pub struct SignIn {
    pub db: TestDb,
    pub github: GitHub,
    pub server: Server,
    /// The server's public address, e.g. `http://skald.test:41234`, without a trailing slash.
    pub public: String,
}

/// An account as the session gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub github_login: String,
}

impl Account {
    /// The account in `body`, a `GET /session` answer.
    pub fn of(body: &Value) -> Self {
        let id = body["account"].as_str().expect("an account ID").to_owned();
        assert!(is_uuid_v7(&id), "account ID {id:?} is not a UUIDv7");
        let github_login = body["github_login"].as_str().expect("a GitHub login");
        Account {
            id,
            github_login: github_login.to_owned(),
        }
    }
}

impl SignIn {
    /// A server whose public address is plain HTTP under `skald.test`: a browser's address.
    pub fn start() -> Self {
        Self::start_with(&[], CLIENT_SECRET)
    }

    /// As [`SignIn::start`], with the extra arguments `args` and the client secret `secret`.
    pub fn start_with(args: &[&str], secret: &str) -> Self {
        Self::new(|port| format!("http://skald.test:{port}"), args, secret)
    }

    /// A server whose public address is plain HTTP under `127.0.0.1`, where the test itself sends
    /// requests.
    pub fn start_local() -> Self {
        Self::new(
            |port| format!("http://127.0.0.1:{port}"),
            &[],
            CLIENT_SECRET,
        )
    }

    /// A server whose public address is `https://skald.test:<port>`, an HTTPS proxy of `browser`'s
    /// driver in front of it, as a server behind a TLS-terminating proxy.
    pub fn start_https(browser: &Browser) -> Self {
        Self::new(
            |port| {
                let proxy = browser.tls_proxy(&format!("http://127.0.0.1:{port}"));
                format!("https://skald.test:{proxy}")
            },
            &[],
            CLIENT_SECRET,
        )
    }

    /// A server whose public address `public` gives for the port the server listens on.
    fn new(public: impl Fn(u16) -> String, args: &[&str], secret: &str) -> Self {
        let db = TestDb::create();
        let (github, server, public) = on_free_port(Duration::from_secs(30), |port| {
            let public = public(port);
            let github = GitHub::start(App {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                callback: Some(format!("{public}{CALLBACK}")),
            });
            let mut all = github_settings(&public, &github);
            all.extend_from_slice(args);
            let server = Server::try_launch(
                &db,
                &format!("127.0.0.1:{port}"),
                &all,
                &[("SKALD_GITHUB_CLIENT_SECRET", secret)],
            )?;
            Ok((github, server, public))
        });
        SignIn {
            db,
            github,
            server,
            public,
        }
    }

    /// Adds a GitHub user to the stand-in and gives it.
    pub fn user(&self, id: u64, login: &str) -> User {
        let user = User::new(id, login);
        self.github.put_user(user.clone());
        user
    }

    /// The sign-in start, with the return address `return_to` if given.
    pub fn start_url(&self, return_to: Option<&str>) -> String {
        match return_to {
            Some(to) => {
                let query = serde_urlencoded::to_string([("return_to", to)]).expect("a query");
                format!("{}{START}?{query}", self.public)
            }
            None => format!("{}{START}", self.public),
        }
    }

    /// Signs `page`'s browser in to GitHub as `user`, then to the server with GitHub.
    pub fn sign_in(&self, page: &Page, user: &User) -> Visit {
        self.github_session(page, user);
        page.goto(&self.start_url(None))
    }

    /// Signs `page`'s browser in to GitHub as `user`, as a person does before a sign-in.
    pub fn github_session(&self, page: &Page, user: &User) {
        let visit = page.goto(&self.github.sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
    }

    /// Runs a sign-in of `user` in `page`'s browser up to GitHub's page that asks to authorize the
    /// app; gives the callback URL its link leads to, with `code` and `state`, not yet opened.
    pub fn held_callback(&self, page: &Page, user: &User) -> String {
        let visit = page.goto(&self.github.asking_sign_in_url(user.id));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        let visit = page.goto(&self.start_url(None));
        self.authorize_link(page, &visit)
    }

    /// The link of GitHub's page that asks to authorize the app, where `visit` ended.
    pub fn authorize_link(&self, page: &Page, visit: &Visit) -> String {
        let authorize = format!("{}/login/oauth/authorize?", self.github.url());
        assert!(visit.url.starts_with(&authorize), "at GitHub: {visit:?}");
        let link = page.evaluate(r#"document.getElementById("authorize").href"#);
        let link = link.as_str().expect("a link to authorize the app");
        let callback = format!("{}{CALLBACK}?", self.public);
        assert!(link.starts_with(&callback), "to the callback: {link}");
        link.to_owned()
    }

    /// `GET /session` from a page of the server in `page`'s browser: the status and the body.
    pub fn session(&self, page: &Page) -> (u16, Value) {
        let visit = page.goto(&format!("{}/health", self.public));
        assert_eq!(visit.status, Some(200), "GET /health: {visit:?}");
        let answer = page.evaluate(&format!(
            r#"fetch("{SESSION}", {{ headers: {{ "{header}": "{version}" }} }})
                .then(async (response) => ({{ status: response.status, body: await response.text() }}))"#,
            header = skald_proto::VERSION_HEADER,
            version = skald_proto::PROTOCOL_VERSION,
        ));
        let status = answer["status"].as_u64().expect("a status") as u16;
        let body = answer["body"].as_str().expect("a body");
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    }

    /// The account of `page`'s session; fails without one.
    pub fn account(&self, page: &Page) -> Account {
        let (status, body) = self.session(page);
        assert_eq!(status, 200, "GET {SESSION}: {body}");
        Account::of(&body)
    }

    /// Checks that `page`'s browser has no session on the server.
    pub fn assert_signed_out(&self, page: &Page) {
        let (status, body) = self.session(page);
        assert_eq!(status, 401, "GET {SESSION}: {body}");
    }

    /// The rows of the database that hold `user`'s GitHub ID or login.
    pub fn rows_of(&self, user: &User) -> Vec<String> {
        let id = user.id.to_string();
        let mut rows = self.db.rows();
        rows.retain(|row| row.contains(&user.login) || holds_number(row, &id));
        rows
    }
}

/// Whether `text` holds the decimal `number`, not as a part of a longer one.
fn holds_number(text: &str, number: &str) -> bool {
    text.match_indices(number).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + number.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// Whether `id` is a UUIDv7 in the lowercase hyphenated form.
pub fn is_uuid_v7(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(at, byte)| match at {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
        && bytes[14] == b'7'
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}
