//! A `skald-server` set up for GitHub sign-in against the GitHub stand-in, and a person signing in
//! to it in a browser.

use std::time::Duration;

use serde_json::Value;
use skald_github_stand_in::{App, GitHub, User};

use super::browser::{Browser, Cookie, Page, Submission, Visit};
use super::{Response, Server, TestDb, get_url, on_free_port};

/// The fake OAuth App's client ID and secret.
pub const CLIENT_ID: &str = "Ov23li0standin0client";
pub const CLIENT_SECRET: &str = "standin-secret-5c0f1e7a9b2d4c6e8f0a1b3c5d7e9f1a";
/// The environment variable that holds the client secret.
pub const SECRET_ENV: &str = "SKALD_GITHUB_CLIENT_SECRET";

/// Where the browser starts signing in: it takes the return address as `return_to`.
pub const START: &str = "/auth/github";
/// The GitHub callback, under the server's public address.
pub const CALLBACK: &str = "/auth/github/callback";
/// The session's account: `{"account": <ID>, "github_login": <login>}`, `401` without a session.
pub const SESSION: &str = "/session";

/// The server's settings that name the public address, the OAuth App and GitHub's addresses.
pub fn github_settings<'a>(public: &'a str, github: &'a GitHub) -> Vec<&'a str> {
    let mut settings = vec!["--public-url", public];
    settings.extend(github_args(github));
    settings
}

/// The server's settings that name the OAuth App and GitHub's addresses.
pub fn github_args(github: &GitHub) -> [&str; 6] {
    [
        "--github-client-id",
        CLIENT_ID,
        "--github-url",
        github.url(),
        "--github-api-url",
        github.url(),
    ]
}

/// The GitHub stand-in with the OAuth App, whose callback is under the server's public address
/// `public`.
pub fn github_stand_in(public: &str) -> GitHub {
    GitHub::start(App {
        client_id: CLIENT_ID.to_owned(),
        client_secret: CLIENT_SECRET.to_owned(),
        callback: Some(format!("{public}{CALLBACK}")),
    })
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

    /// A server whose public address is `public`, such as that of a front before it, with the
    /// extra arguments `args`.
    pub fn behind(public: &str, args: &[&str]) -> Self {
        Self::new(|_| public.to_owned(), args, CLIENT_SECRET)
    }

    /// As [`SignIn::start`], on `db`, such as a database that holds what an earlier server made.
    pub fn on(db: TestDb) -> Self {
        Self::new_on(
            db,
            |port| format!("http://skald.test:{port}"),
            &[],
            CLIENT_SECRET,
        )
    }

    /// As [`SignIn::on`], with the extra arguments `args` and the environment variables `envs`.
    pub fn on_with(db: TestDb, args: &[&str], envs: &[(&str, &str)]) -> Self {
        Self::new_with(
            db,
            |port| format!("http://skald.test:{port}"),
            args,
            CLIENT_SECRET,
            envs,
        )
    }

    /// A server whose public address `public` gives for the port the server listens on.
    fn new(public: impl Fn(u16) -> String, args: &[&str], secret: &str) -> Self {
        Self::new_on(TestDb::create(), public, args, secret)
    }

    /// As [`SignIn::new`], on `db`.
    fn new_on(db: TestDb, public: impl Fn(u16) -> String, args: &[&str], secret: &str) -> Self {
        Self::new_with(db, public, args, secret, &[])
    }

    /// As [`SignIn::new_on`], with the environment variables `envs`.
    fn new_with(
        db: TestDb,
        public: impl Fn(u16) -> String,
        args: &[&str],
        secret: &str,
        envs: &[(&str, &str)],
    ) -> Self {
        let (github, server, public) = on_free_port(Duration::from_secs(30), |port| {
            let public = public(port);
            let github = github_stand_in(&public);
            let mut all = github_settings(&public, &github);
            all.extend_from_slice(args);
            let mut all_envs = vec![(SECRET_ENV, secret)];
            all_envs.extend_from_slice(envs);
            let server = Server::try_launch(&db, &format!("127.0.0.1:{port}"), &all, &all_envs)?;
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
        start_url(&self.public, START, return_to)
    }

    /// Runs a sign-in of `user` by plain HTTP, straight to the server whatever its public address,
    /// up to GitHub's redirect back: the start's answer and the callback URL on the server.
    pub fn to_callback(&self, user: &User) -> (Response, String) {
        to_github_callback(&self.server, &self.github, &self.public, user)
    }

    /// Signs `page`'s browser in to GitHub as `user`, then to the server with GitHub.
    pub fn sign_in(&self, page: &Page, user: &User) -> Visit {
        self.github_session(page, user);
        page.goto(&self.start_url(None))
    }

    /// Signs `page`'s browser in to GitHub as `user`, as a person does before a sign-in.
    pub fn github_session(&self, page: &Page, user: &User) {
        github_session(page, &self.github, user);
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
        github_authorize_link(&self.github, &self.public, page, visit)
    }

    /// `GET /session` from a page of the server in `page`'s browser: the status and the body.
    pub fn session(&self, page: &Page) -> (u16, Value) {
        page_session(page, &self.public)
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
        github_rows(&self.db, user)
    }
}

/// Signs `page`'s browser in to `github` as `user`, as a person does before a sign-in.
pub fn github_session(page: &Page, github: &GitHub, user: &User) {
    let visit = page.goto(&github.sign_in_url(user.id));
    assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
}

/// The link of `github`'s page that asks to authorize the app, where `visit` ended in `page`'s
/// browser: the callback of the server at the public address `public`, with `code` and `state`.
pub fn github_authorize_link(github: &GitHub, public: &str, page: &Page, visit: &Visit) -> String {
    let authorize = format!("{}/login/oauth/authorize", github.url());
    authorize_link(page, visit, &authorize, &format!("{public}{CALLBACK}"))
}

/// The link of the provider's page that asks to continue, where `visit` ended in `page`'s browser:
/// that page is at `authorize`, the provider's authorization endpoint, and the link leads to
/// `callback`, the server's, with `code` and `state`.
pub fn authorize_link(page: &Page, visit: &Visit, authorize: &str, callback: &str) -> String {
    let authorize = format!("{authorize}?");
    assert!(
        visit.url.starts_with(&authorize),
        "at the provider: {visit:?}"
    );
    let link = page.evaluate(r#"document.getElementById("authorize").href"#);
    let link = link.as_str().expect("a link to continue");
    let callback = format!("{callback}?");
    assert!(link.starts_with(&callback), "to the callback: {link}");
    link.to_owned()
}

/// The rows of `db` that hold the GitHub ID or login of `user`.
pub fn github_rows(db: &TestDb, user: &User) -> Vec<String> {
    let id = user.id.to_string();
    let mut rows = db.rows();
    rows.retain(|row| row.contains(&user.login) || holds_number(row, &id));
    rows
}

/// Runs a GitHub sign-in of `user` by plain HTTP at `server`, whose public address is `public`,
/// straight to it, against `github`, up to GitHub's redirect back: the start's answer and the
/// callback URL on the server.
pub fn to_github_callback(
    server: &Server,
    github: &GitHub,
    public: &str,
    user: &User,
) -> (Response, String) {
    to_callback(server, public, START, &github.sign_in_url(user.id))
}

/// Runs a sign-in by plain HTTP at `server`, whose public address is `public`, straight to it, from
/// the sign-in start `start`, such as `/auth/github`, as the person whom the provider's
/// `sign_in_url` signs in, up to the provider's redirect back: the start's answer and the callback
/// URL on the server.
pub fn to_callback(
    server: &Server,
    public: &str,
    start: &str,
    sign_in_url: &str,
) -> (Response, String) {
    let started = server.get(start);
    assert_eq!(
        started.status,
        302,
        "GET {start}: {}",
        String::from_utf8_lossy(&started.body)
    );
    let mut provider = Jar::default();
    provider.take(&get_url(sign_in_url, &[]));
    let authorize = started
        .header("location")
        .expect("a redirect to the provider");
    let back = get_url(authorize, &[("cookie", &provider.header())]);
    let callback = back.header("location").expect("the provider goes back");
    let callback = callback.replacen(public, server.base(), 1);
    (started, callback)
}

/// The sign-in start `start`, such as `/auth/github`, under the public address `public`, with the
/// return address `return_to` if given.
pub fn start_url(public: &str, start: &str, return_to: Option<&str>) -> String {
    match return_to {
        Some(to) => {
            let query = serde_urlencoded::to_string([("return_to", to)]).expect("a query");
            format!("{public}{start}?{query}")
        }
        None => format!("{public}{start}"),
    }
}

/// `GET /session` from a page of the server at the public address `public` in `page`'s browser:
/// the status and the body.
pub fn page_session(page: &Page, public: &str) -> (u16, Value) {
    page_get(page, public, SESSION)
}

/// `GET <path>` of the server's API, stating the protocol version, from a page of the server at the
/// public address `public` in `page`'s browser: the status and the JSON body, `null` if none.
pub fn page_get(page: &Page, public: &str, path: &str) -> (u16, Value) {
    let visit = page.goto(&format!("{public}/health"));
    assert_eq!(visit.status, Some(200), "GET /health: {visit:?}");
    let answer = page.evaluate(&format!(
        r#"fetch("{path}", {{ headers: {{ "{header}": "{version}" }} }})
            .then(async (response) => ({{ status: response.status, body: await response.text() }}))"#,
        header = skald_proto::VERSION_HEADER,
        version = skald_proto::PROTOCOL_VERSION,
    ));
    let status = answer["status"].as_u64().expect("a status") as u16;
    let body = answer["body"].as_str().expect("a body");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// Opens `callback`, a sign-in's callback held before, in `page`'s browser; gives the cookies of
/// `host`, the server's, that it set or changed: the session's.
pub fn cookies_set_by(page: &Page, host: &str, callback: &str) -> Vec<Cookie> {
    let before = page.cookies_of(host);
    page.goto(callback);
    let mut set = page.cookies_of(host);
    set.retain(|cookie| !before.contains(cookie));
    assert!(!set.is_empty(), "the callback sets the session's cookie");
    set
}

/// States a callback's `state` is not: longer, with its last character changed, cut in half, and
/// another's.
pub fn wrong_states(state: &str) -> Vec<String> {
    let mut flipped = state.to_owned().into_bytes();
    let last = flipped.last_mut().expect("a state is not empty");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let flipped = String::from_utf8(flipped).expect("ASCII");
    vec![
        format!("{state}A"),
        flipped,
        state[..state.len() / 2].to_owned(),
        "0123456789abcdef0123456789abcdef".to_owned(),
    ]
}

/// Return addresses off a server under `host`, as a sign-in's `return_to`; `elsewhere` is a port of
/// that host the server does not listen on.
pub fn off_server(host: &str, elsewhere: u16) -> Vec<String> {
    vec![
        "https://evil.test/".to_owned(),
        "http://evil.test/".to_owned(),
        "//evil.test/".to_owned(),
        "/\\evil.test/".to_owned(),
        "\\\\evil.test/".to_owned(),
        "/\t/evil.test/".to_owned(),
        "https:evil.test/".to_owned(),
        format!("http://{host}.evil.test/"),
        format!("http://{host}:{elsewhere}/"),
        "javascript:alert(document.cookie)".to_owned(),
    ]
}

/// Checks that a sign-in's cookies of `host` go only over HTTPS: `state`, the cookies that bind a
/// sign-in's `state`, probed at the sign-in's `callback` path, and `session`, the session's, probed
/// at `/`, `/session` and `callback`, each with the page of the browser that holds them. A probe
/// opens a path on a plain and an HTTPS echo server of `browser` under `host`: no plain request
/// carries a cookie, and each cookie goes with some HTTPS one.
pub fn assert_only_over_https(
    browser: &Browser,
    (host, callback): (&str, &str),
    state: (&Page, &[Cookie]),
    session: (&Page, &[Cookie]),
) {
    let plain = browser.echo(false);
    let secure = browser.echo(true);
    let probes = [
        (state, vec![callback], "state"),
        (session, vec!["/", SESSION, callback], "session"),
    ];
    for ((page, cookies), paths, probe) in probes {
        let query = format!("probe={probe}");
        for path in &paths {
            for (port, scheme) in [(plain, "http"), (secure, "https")] {
                page.goto(&format!("{scheme}://{host}:{port}{path}?{query}"));
            }
        }
        for hit in browser.wait_for_hits(plain, &query, paths.len()) {
            assert_eq!(hit.cookie, None, "plain HTTP carries no cookie: {hit:?}");
        }
        let over_https = browser.wait_for_hits(secure, &query, paths.len());
        for cookie in cookies {
            assert!(
                over_https.iter().any(|hit| hit.carries(cookie)),
                "HTTPS carries {cookie:?}: {over_https:?}"
            );
        }
    }
}

/// Cookies as a client that follows no rules but name and value keeps them.
#[derive(Clone, Default)]
pub struct Jar(Vec<(String, String)>);

impl Jar {
    /// Keeps the cookies `response` sets, replacing those of the same name.
    pub fn take(&mut self, response: &Response) {
        for set in response.header_values("set-cookie") {
            let pair = set.split(';').next().unwrap_or_default();
            let Some((name, value)) = pair.trim().split_once('=') else {
                continue;
            };
            self.0.retain(|(kept, _)| kept != name);
            self.0.push((name.to_owned(), value.to_owned()));
        }
    }

    /// The `Cookie` header that carries them.
    pub fn header(&self) -> String {
        let pairs: Vec<_> = self
            .0
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        pairs.join("; ")
    }
}

/// The `Set-Cookie` value of `response` for the cookie `name`.
pub fn set_cookie<'a>(response: &'a Response, name: &str) -> &'a str {
    response
        .header_values("set-cookie")
        .into_iter()
        .find(|cookie| cookie.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no cookie {name}: {:?}", response.headers))
}

/// The `name=value` of the cookie `name` that `response` sets.
pub fn cookie_set(response: &Response, name: &str) -> String {
    let cookie = set_cookie(response, name);
    cookie.split(';').next().unwrap_or_default().to_owned()
}

/// The value of the query parameter `name` of `url`.
pub fn param(url: &str, name: &str) -> Option<String> {
    query(url)
        .into_iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

/// `url` with its query parameter `name` set to `value`, or removed for `None`.
pub fn with_param(url: &str, name: &str, value: Option<&str>) -> String {
    let base = url.split('?').next().unwrap_or(url);
    let mut pairs = query(url);
    pairs.retain(|(key, _)| key != name);
    if let Some(value) = value {
        pairs.push((name.to_owned(), value.to_owned()));
    }
    let encoded = serde_urlencoded::to_string(&pairs).expect("a query");
    format!("{base}?{encoded}")
}

fn query(url: &str) -> Vec<(String, String)> {
    let query = url
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default();
    serde_urlencoded::from_str(query).expect("a query")
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

/// Opens `site`, a page of another site, in `page`'s browser and submits from it the form that
/// `submission` describes, as that site can; gives the visit.
pub fn submit_from_site(page: &Page, site: &str, submission: &Submission) -> Visit {
    page.goto(site);
    page.navigate_by(&format!(
        r#"setTimeout(() => {{
            const form = document.createElement("form");
            form.method = {method};
            form.action = {action};
            for (const [name, value] of {fields}) {{
                const field = document.createElement("input");
                field.type = "hidden";
                field.name = name;
                field.value = value;
                form.append(field);
            }}
            document.body.append(form);
            form.submit();
        }}, 0)"#,
        method = serde_json::to_string(&submission.method).expect("JSON"),
        action = serde_json::to_string(&submission.action).expect("JSON"),
        fields = serde_json::to_string(&submission.fields).expect("JSON"),
    ))
}

/// Opens `site`, a page of another site, in `page`'s browser and follows a link from it to `url`;
/// gives the visit.
pub fn follow_from_site(page: &Page, site: &str, url: &str) -> Visit {
    page.goto(site);
    page.navigate_by(&format!(
        r#"setTimeout(() => {{ location.href = {}; }}, 0)"#,
        serde_json::to_string(url).expect("JSON")
    ))
}

/// What `submission` sends, as a link: its action with its fields as the query.
pub fn as_link(submission: &Submission) -> String {
    let query = serde_urlencoded::to_string(&submission.fields).expect("a query");
    let action = submission.action.split('?').next().unwrap_or_default();
    format!("{action}?{query}")
}

/// Whether `text` is an S256 PKCE challenge (RFC 7636, §4.2): 43 characters of base64url.
pub fn is_s256_challenge(text: &str) -> bool {
    text.len() == 43
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
