//! A `skald-server` set up for Google sign-in against the local stand-in for an OIDC provider, which
//! plays Google, and GitHub sign-in against the GitHub stand-in beside it if asked; a person signing
//! in to it in a browser. No test calls a real Google endpoint.
//!
//! The server, as these tests use it:
//! - settings: `--google-client-id <ID>`, the client secret only in the environment variable
//!   `SKALD_GOOGLE_CLIENT_SECRET`, and `--google-issuer <URL>`, Google's
//!   `https://accounts.google.com` by default; the public address `--public-url` and the lifetime
//!   of a sign-in's `state` `--sign-in-state-lifetime`, as GitHub sign-in has them;
//! - `GET /auth/google`, with an optional `return_to`: sends the browser to the provider's
//!   authorization endpoint with a `state` bound to that browser by a cookie;
//! - `GET /auth/google/callback`: the provider's redirect URI, `<public URL>/auth/google/callback`,
//!   which the client registered with the stand-in has; it ends in a web session as GitHub's
//!   callback does;
//! - `GET /session`: the session's account, `{"account": <ID>, "username": <username>}`, with
//!   `github_login` only while the account has a GitHub sign-in method.

use std::time::Duration;

use super::accounts::Who;
use super::browser::{Browser, Page, Visit};
use super::sign_in::{
    CALLBACK as GITHUB_CALLBACK, CLIENT_ID as GITHUB_CLIENT_ID,
    CLIENT_SECRET as GITHUB_CLIENT_SECRET, Jar, SESSION, page_session, start_url,
    to_github_callback,
};
use super::{Response, Server, TestDb, get_url, on_free_port};
use skald_github_stand_in::{App, GitHub, User};
use skald_oidc_stand_in::{Client, Key, Person, Provider};

/// The server's OAuth client at the provider.
pub const CLIENT_ID: &str = "skald-stand-in-google-client";
pub const CLIENT_SECRET: &str = "stand-in-google-secret-7c1e9a3f5b2d4e6a8c0f";
/// The environment variable that holds the client secret.
pub const SECRET_ENV: &str = "SKALD_GOOGLE_CLIENT_SECRET";
/// A client secret other than the one the stand-in's client has.
pub const WRONG_SECRET: &str = "stand-in-google-secret-0000000000wrong00000";

/// Where the browser starts a Google sign-in: it takes the return address as `return_to`.
pub const START: &str = "/auth/google";
/// Google's callback, under the server's public address.
pub const CALLBACK: &str = "/auth/google/callback";
/// Where the browser starts a GitHub sign-in.
pub const GITHUB_START: &str = "/auth/github";

/// The host of the server's public address in the browser tests.
pub const HOST: &str = "skald.test";

/// How the server is set up.
#[derive(Default)]
pub struct Setup<'a> {
    /// The server's public address is `https://skald.test:<port>`, an HTTPS proxy of this
    /// browser's driver in front of it.
    pub https: Option<&'a Browser>,
    /// The server's public address is `http://127.0.0.1:<port>`, where the test itself sends
    /// requests, instead of `http://skald.test:<port>`.
    pub local: bool,
    /// The server's public address, such as a front's in front of it, instead of one of the above.
    pub public: Option<&'a str>,
    /// The server has GitHub's settings too, against the GitHub stand-in.
    pub github: bool,
    /// The server has no Google settings.
    pub no_google: bool,
    /// The issuer the server has, if not the stand-in's, such as a front's in front of it.
    pub issuer: Option<&'a str>,
    /// The provider is down when the server starts.
    pub down: bool,
    /// The client secret the server has, if not the client's.
    pub secret: Option<&'a str>,
    /// Extra arguments.
    pub args: &'a [&'a str],
}

/// A server with Google sign-in through the OIDC stand-in, on its own database.
pub struct Google {
    pub db: TestDb,
    pub provider: Provider,
    /// The GitHub stand-in, when the server has GitHub's settings too.
    pub github: Option<GitHub>,
    pub server: Server,
    /// The server's public address, e.g. `http://skald.test:41234`, without a trailing slash.
    pub public: String,
}

impl Google {
    /// A server with Google's settings only, whose public address is plain HTTP under `skald.test`.
    pub fn start() -> Self {
        Self::launch(Setup::default())
    }

    /// A server set up as `setup` says.
    pub fn launch(setup: Setup) -> Self {
        let mut provider = Provider::start(&[Key::rsa()]);
        if setup.down {
            provider.stop();
        }
        let issuer = setup.issuer.unwrap_or(provider.issuer()).to_owned();
        let db = TestDb::create();
        let (github, server, public) = on_free_port(Duration::from_secs(30), |port| {
            let public = if let Some(public) = setup.public {
                public.to_owned()
            } else if let Some(browser) = setup.https {
                let proxy = browser.tls_proxy(&format!("http://127.0.0.1:{port}"));
                format!("https://{HOST}:{proxy}")
            } else if setup.local {
                format!("http://127.0.0.1:{port}")
            } else {
                format!("http://{HOST}:{port}")
            };
            provider.register(Client {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                redirect_uri: format!("{public}{CALLBACK}"),
            });
            let github = setup.github.then(|| {
                GitHub::start(App {
                    client_id: GITHUB_CLIENT_ID.to_owned(),
                    client_secret: GITHUB_CLIENT_SECRET.to_owned(),
                    callback: Some(format!("{public}{GITHUB_CALLBACK}")),
                })
            });
            let mut args = vec!["--public-url", public.as_str()];
            let mut envs = Vec::new();
            if !setup.no_google {
                args.extend(["--google-client-id", CLIENT_ID, "--google-issuer", &issuer]);
                envs.push((SECRET_ENV, setup.secret.unwrap_or(CLIENT_SECRET)));
            }
            if let Some(github) = &github {
                args.extend(["--github-client-id", GITHUB_CLIENT_ID]);
                args.extend([
                    "--github-url",
                    github.url(),
                    "--github-api-url",
                    github.url(),
                ]);
                envs.push(("SKALD_GITHUB_CLIENT_SECRET", GITHUB_CLIENT_SECRET));
            }
            args.extend_from_slice(setup.args);
            let server = Server::try_launch(&db, &format!("127.0.0.1:{port}"), &args, &envs)?;
            Ok((github, server, public))
        });
        Google {
            db,
            provider,
            github,
            server,
            public,
        }
    }

    /// Adds a person to the stand-in and gives them.
    pub fn person(&self, subject: &str, email: &str) -> Person {
        let person = Person::new(subject, email);
        self.provider.put_person(person.clone());
        person
    }

    /// Adds a GitHub user with the public email `email` to the GitHub stand-in and gives them.
    pub fn github_user(&self, id: u64, login: &str, email: &str) -> User {
        let user = User {
            email: Some(email.to_owned()),
            ..User::new(id, login)
        };
        self.github().put_user(user.clone());
        user
    }

    pub fn github(&self) -> &GitHub {
        self.github
            .as_ref()
            .expect("the server has GitHub's settings")
    }

    /// The Google sign-in start, with the return address `return_to` if given.
    pub fn start_url(&self, return_to: Option<&str>) -> String {
        start_url(&self.public, START, return_to)
    }

    /// Signs `page`'s browser in to the provider as `person`, as a person does before a sign-in.
    pub fn provider_session(&self, page: &Page, person: &Person) {
        let visit = page.goto(&self.provider.sign_in_url(&person.subject));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
    }

    /// Signs `page`'s browser in to the provider as `person`, then to the server with Google.
    pub fn sign_in(&self, page: &Page, person: &Person) -> Visit {
        self.provider_session(page, person);
        page.goto(&self.start_url(None))
    }

    /// Signs `page`'s browser in to GitHub as `user`, then to the server with GitHub.
    pub fn github_sign_in(&self, page: &Page, user: &User) {
        let visit = page.goto(&self.github().sign_in_url(user.id));
        assert_eq!(
            visit.status,
            Some(200),
            "GitHub stand-in sign-in: {visit:?}"
        );
        page.goto(&format!("{}{GITHUB_START}", self.public));
    }

    /// Runs a sign-in of `person` in `page`'s browser up to the provider's page that asks to
    /// continue; gives the callback URL its link leads to, with `code` and `state`, not yet opened.
    pub fn held_callback(&self, page: &Page, person: &Person) -> String {
        let visit = page.goto(&self.provider.asking_sign_in_url(&person.subject));
        assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
        let visit = page.goto(&self.start_url(None));
        self.authorize_link(page, &visit)
    }

    /// The link of the provider's page that asks to continue, where `visit` ended.
    pub fn authorize_link(&self, page: &Page, visit: &Visit) -> String {
        let authorize = format!("{}?", self.provider.authorize_url());
        assert!(
            visit.url.starts_with(&authorize),
            "at the provider: {visit:?}"
        );
        let link = page.evaluate(r#"document.getElementById("authorize").href"#);
        let link = link.as_str().expect("a link to continue");
        let callback = format!("{}{CALLBACK}?", self.public);
        assert!(link.starts_with(&callback), "to the callback: {link}");
        link.to_owned()
    }

    /// Runs a sign-in of `person` by plain HTTP, straight to the server whatever its public
    /// address, up to the provider's redirect back: the cookies the start set and the callback URL
    /// on the server.
    pub fn to_callback(&self, person: &Person) -> (Jar, String) {
        let start = self.server.get(START);
        assert_eq!(start.status, 302, "GET {START}: {}", text(&start));
        let mut jar = Jar::default();
        jar.take(&start);
        let mut provider = Jar::default();
        provider.take(&get_url(&self.provider.sign_in_url(&person.subject), &[]));
        let authorize = start
            .header("location")
            .expect("a redirect to the provider");
        let back = get_url(authorize, &[("cookie", &provider.header())]);
        let callback = back.header("location").expect("the provider goes back");
        let callback = callback.replacen(&self.public, self.server.base(), 1);
        (jar, callback)
    }

    /// Runs a GitHub sign-in of `user` by plain HTTP, as `to_callback` does a Google one: the
    /// cookies the start set and the callback URL on the server.
    pub fn github_to_callback(&self, user: &User) -> (Jar, String) {
        let (start, callback) = to_github_callback(&self.server, self.github(), &self.public, user);
        let mut jar = Jar::default();
        jar.take(&start);
        (jar, callback)
    }

    /// Opens `callback` by plain HTTP with the cookies of `jar`; gives the cookies afterwards.
    pub fn finish(&self, jar: &Jar, callback: &str) -> Jar {
        let done = get_url(callback, &[("cookie", &jar.header())]);
        assert_eq!(done.status, 200, "GET {callback}: {}", text(&done));
        let mut jar = jar.clone();
        jar.take(&done);
        jar
    }

    /// `GET /session` by plain HTTP with the cookies of `jar`.
    pub fn session_of(&self, jar: &Jar) -> Response {
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        self.server.get_with(
            SESSION,
            &[
                (skald_proto::VERSION_HEADER, &version),
                ("cookie", &jar.header()),
            ],
        )
    }

    /// The account of `page`'s session; fails without one.
    pub fn account(&self, page: &Page) -> Who {
        let (status, body) = page_session(page, &self.public);
        assert_eq!(status, 200, "GET {SESSION}: {body}");
        Who::of_body(&body)
    }

    /// Checks that `page`'s browser has no session on the server.
    pub fn assert_signed_out(&self, page: &Page) {
        let (status, body) = page_session(page, &self.public);
        assert_eq!(status, 401, "GET {SESSION}: {body}");
    }

    /// Signs `person` in on a new page of `browser` and gives the account; fails if no one is
    /// signed in.
    pub fn signed_up(&self, browser: &Browser, person: &Person) -> Who {
        let page = browser.page();
        self.sign_in(&page, person);
        let who = self.account(&page);
        assert_eq!(who.github_login, None, "a Google account: {who:?}");
        who
    }

    /// The rows of the database that hold `person`'s subject or email: those of their account.
    pub fn rows_of(&self, person: &Person) -> Vec<String> {
        let mut rows = self.db.rows();
        rows.retain(|row| row.contains(&person.subject) || row.contains(&person.email));
        rows
    }

    /// Checks that `person` has no account: no row holds their subject or email.
    #[track_caller]
    pub fn assert_no_account(&self, person: &Person, what: &str) {
        assert_eq!(self.rows_of(person), Vec::<String>::new(), "{what}");
    }
}

/// The body of `response` as text.
pub fn text(response: &Response) -> String {
    String::from_utf8_lossy(&response.body).into_owned()
}

/// The value of the query parameter `name` of `url`.
pub fn param(url: &str, name: &str) -> Option<String> {
    super::sign_in::param(url, name)
}
