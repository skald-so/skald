//! The device flow (RFC 8628) on a `skald-server` set up for GitHub sign-in against the GitHub
//! stand-in: a machine's requests, each checked against the OpenAPI spec, and its person on the
//! verification page in a browser.
//!
//! The person's page: a textbox for the code, a button named "Approve" and one named "Deny"; when
//! the page asks for the code first, its only button submits it and the next page has both. Each
//! is an HTML form whose submission navigates; a refused code answers with a `4xx`.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_github_stand_in::User;

use super::browser::{APPROVE, CODE_FIELD, DENY, Page, SUBMIT, Visit};
use super::sign_in::{Account, CLIENT_SECRET, SESSION, SignIn, cookie_set};
use super::spec::{operation_violations, spec};
use super::{Response, delete_url, get_url, post_form, send};

/// Device authorization (RFC 8628, §3.1): `POST`, a form of `client_id`.
pub const DEVICE_AUTHORIZATION: &str = "/auth/device";
/// The token endpoint (RFC 8628, §3.4): `POST`, a form of `grant_type`, `device_code` and
/// `client_id`.
pub const TOKEN: &str = "/auth/token";
/// The `client_id` of `skald`, the CLI.
pub const CLIENT_ID: &str = "skald";
/// The device flow's `grant_type`.
pub const GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The settings: how long a device code and its user code work, and the polling interval, in
/// seconds.
pub const LIFETIME: &str = "--device-code-lifetime";
pub const INTERVAL: &str = "--device-poll-interval";

/// What device authorization gives (RFC 8628, §3.2).
#[derive(Clone, Debug)]
pub struct Grant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// A poll's answer: a session token, or the `error` of a `400`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Token(String),
    Error(String),
}

/// A server with GitHub sign-in through the stand-in and the device flow, on its own database.
pub struct DeviceFlow {
    pub stand: SignIn,
    spec: Value,
}

impl DeviceFlow {
    /// A server whose codes poll every second.
    pub fn start() -> Self {
        Self::start_with(&[INTERVAL, "1"])
    }

    /// A server with the extra arguments `args`, as [`SignIn::start_with`] starts it.
    pub fn start_with(args: &[&str]) -> Self {
        Self::of(SignIn::start_with(args, CLIENT_SECRET))
    }

    /// The device flow of `stand`'s server, which it starts with no extra arguments.
    pub fn of(stand: SignIn) -> Self {
        DeviceFlow {
            stand,
            spec: spec(),
        }
    }

    /// Checks that `response` to `<method> <path>` conforms to the OpenAPI spec.
    pub fn conforms(&self, method: &str, path: &str, response: &Response) {
        let violations = operation_violations(&self.spec, method, path, response);
        assert_eq!(violations, Vec::<String>::new());
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.stand.server.base())
    }

    /// `POST <path>` of `fields` straight to the server, stating the protocol version; checked
    /// against the spec.
    pub fn post(&self, path: &str, fields: &[(&str, &str)]) -> Response {
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        let headers = [(skald_proto::VERSION_HEADER, version.as_str())];
        let response = post_form(&self.url(path), &headers, fields);
        self.conforms("post", path, &response);
        response
    }

    /// Device authorization, as a machine asks for it.
    pub fn authorize_response(&self) -> Response {
        self.post(DEVICE_AUTHORIZATION, &[("client_id", CLIENT_ID)])
    }

    /// A new device code and its user code.
    pub fn authorize(&self) -> Grant {
        let response = self.authorize_response();
        assert_eq!(response.status, 200, "{}", text(&response));
        let body = json(&response);
        let string = |name: &str| {
            body[name]
                .as_str()
                .unwrap_or_else(|| panic!("{name} in {body}"))
                .to_owned()
        };
        let number = |name: &str| {
            body[name]
                .as_u64()
                .unwrap_or_else(|| panic!("{name} in {body}"))
        };
        Grant {
            device_code: string("device_code"),
            user_code: string("user_code"),
            verification_uri: string("verification_uri"),
            verification_uri_complete: string("verification_uri_complete"),
            expires_in: number("expires_in"),
            interval: number("interval"),
        }
    }

    /// The token endpoint's answer to a poll for `device_code`.
    pub fn poll_response(&self, device_code: &str) -> Response {
        self.post(
            TOKEN,
            &[
                ("grant_type", GRANT_TYPE),
                ("device_code", device_code),
                ("client_id", CLIENT_ID),
            ],
        )
    }

    /// A poll for `device_code`.
    pub fn poll(&self, device_code: &str) -> Poll {
        answer(&self.poll_response(device_code))
    }

    /// Polls for `grant` as a client does, `interval` seconds apart, `5` more after each
    /// `slow_down`, until the answer is not `authorization_pending`; for up to 60 s.
    pub fn wait(&self, grant: &Grant) -> Poll {
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut interval = grant.interval;
        loop {
            pause(interval);
            match self.poll(&grant.device_code) {
                Poll::Error(error) if error == "authorization_pending" => {}
                Poll::Error(error) if error == "slow_down" => interval += 5,
                answer => return answer,
            }
            assert!(Instant::now() < deadline, "still pending: {grant:?}");
        }
    }

    /// The session token that `grant` gives once approved.
    pub fn token(&self, grant: &Grant) -> String {
        match self.wait(grant) {
            Poll::Token(token) => token,
            Poll::Error(error) => panic!("no token for {grant:?}: {error}"),
        }
    }

    /// Checks that a poll for `grant`, `interval` seconds after the last one, is still pending.
    pub fn assert_pending(&self, grant: &Grant) {
        pause(grant.interval);
        let answer = self.poll(&grant.device_code);
        assert_eq!(answer, Poll::Error("authorization_pending".to_owned()));
    }

    /// `<method> <path>` straight to the server, `method` in lowercase, with the request headers
    /// `headers` and the JSON `body` if given, stating the protocol version if `stated`; checked
    /// against the spec.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        stated: bool,
    ) -> Response {
        let version = skald_proto::PROTOCOL_VERSION.to_string();
        let mut all = headers.to_vec();
        if stated {
            all.push((skald_proto::VERSION_HEADER, &version));
        }
        let url = self.url(path);
        let response = match body {
            Some(body) => {
                all.push(("content-type", "application/json"));
                send(method, &url, &all, body)
            }
            None if method == "delete" => delete_url(&url, &all),
            None => get_url(&url, &all),
        };
        self.conforms(method, path, &response);
        response
    }

    /// `GET /session` with the request header `Authorization: <authorization>` if given, straight
    /// to the server; checked against the spec.
    pub fn session(&self, authorization: Option<&str>) -> Response {
        let headers: Vec<_> = authorization
            .map(|authorization| ("authorization", authorization))
            .into_iter()
            .collect();
        self.request("get", SESSION, &headers, None, true)
    }

    /// The account a request with the session token `token` acts as.
    pub fn account_of(&self, token: &str) -> Account {
        let response = self.session(Some(&format!("Bearer {token}")));
        assert_eq!(response.status, 200, "{}", text(&response));
        Account::of(&json(&response))
    }

    /// Checks that a request with `authorization` acts as no account.
    pub fn assert_refused(&self, authorization: Option<&str>) {
        let response = self.session(authorization);
        assert_eq!(
            response.status,
            401,
            "{authorization:?}: {}",
            text(&response)
        );
    }

    /// Signs `user` in on the server by plain HTTP, as a browser follows the redirects; gives the
    /// `Cookie` header of the web session.
    pub fn web_cookie(&self, user: &User) -> String {
        let (started, callback) = self.stand.to_callback(user);
        let signed_in = get_url(
            &callback,
            &[("cookie", &cookie_set(&started, "skald_sign_in"))],
        );
        assert_eq!(signed_in.status, 200, "{}", text(&signed_in));
        cookie_set(&signed_in, "skald_session")
    }

    /// Signs `page`'s browser in to the server as `user` and gives their account.
    pub fn signed_in(&self, page: &Page, user: &User) -> Account {
        self.stand.sign_in(page, user);
        self.stand.account(page)
    }

    /// Opens the verification page at `url` in `page`'s browser, signing in with GitHub on the way
    /// if it asks; gives the visit.
    pub fn open(&self, page: &Page, url: &str) -> Visit {
        page.goto_settled(url)
    }

    /// Clicks `button`, [`APPROVE`] or [`DENY`], on the verification page in `page`.
    pub fn decide(&self, page: &Page, button: &str) -> Visit {
        page.click(button)
    }

    /// The person at `page` opens `grant`'s verification page, types its user code and clicks
    /// `button`; gives the last visit: a refused code ends in a `4xx`.
    pub fn enter_and_decide(&self, page: &Page, grant: &Grant, button: &str) -> Visit {
        let visit = self.open(page, &grant.verification_uri);
        if page.count(CODE_FIELD) == 0 {
            return visit;
        }
        page.fill(CODE_FIELD, &grant.user_code);
        proceed(page, button, visit)
    }

    /// The person at `page` opens `grant`'s verification page with the code filled in and clicks
    /// `button`; gives the last visit, as [`DeviceFlow::enter_and_decide`] does.
    pub fn open_and_decide(&self, page: &Page, grant: &Grant, button: &str) -> Visit {
        let visit = self.open(page, &grant.verification_uri_complete);
        proceed(page, button, visit)
    }

    /// The person at `page` types `grant`'s user code on the verification page and approves it;
    /// gives the session token the machine then gets.
    pub fn enter_and_approve(&self, page: &Page, grant: &Grant) -> String {
        assert_ok(&self.enter_and_decide(page, grant, APPROVE));
        self.token(grant)
    }

    /// The person at `page` approves `grant`'s code on the verification page, which has it filled
    /// in; gives the session token the machine then gets.
    pub fn approve(&self, page: &Page, grant: &Grant) -> String {
        self.reach_decision(page, grant);
        assert_ok(&self.decide(page, APPROVE));
        self.token(grant)
    }

    /// The person at `page` opens `grant`'s verification page with the code filled in and gets to
    /// the point where it asks them to approve or deny.
    pub fn reach_decision(&self, page: &Page, grant: &Grant) {
        let visit = self.open(page, &grant.verification_uri_complete);
        assert_eq!(visit.status, Some(200), "{visit:?}");
        if page.count(APPROVE) == 0 {
            let visit = page.click(SUBMIT);
            assert_eq!(visit.status, Some(200), "{visit:?}");
        }
        assert_eq!(page.count(APPROVE), 1, "the page asks to approve");
        assert_eq!(page.count(DENY), 1, "the page offers to deny");
    }
}

/// Clicks `button` on the page in `page`, submitting the code first if the page asks for it; gives
/// the last visit, `opened` if the page has no button.
fn proceed(page: &Page, button: &str, opened: Visit) -> Visit {
    let mut last = opened;
    if page.count(button) == 0 {
        if page.count(SUBMIT) == 0 {
            return last;
        }
        last = page.click(SUBMIT);
        if page.count(button) == 0 {
            return last;
        }
    }
    page.click(button)
}

/// Checks that `visit` ended in a success.
pub fn assert_ok(visit: &Visit) {
    assert!(
        visit
            .status
            .is_some_and(|status| (200..300).contains(&status)),
        "{visit:?}"
    );
}

/// Checks that `visit` ended in a refusal: a `4xx`.
pub fn assert_refused_page(visit: &Visit) {
    assert!(
        visit
            .status
            .is_some_and(|status| (400..500).contains(&status)),
        "refused on the page: {visit:?}"
    );
}

/// A poll's answer in `response`.
pub fn answer(response: &Response) -> Poll {
    let body = json(response);
    match response.status {
        200 => {
            let token = body["access_token"].as_str().expect("a token");
            let kind = body["token_type"].as_str().expect("a token type");
            assert!(kind.eq_ignore_ascii_case("bearer"), "{body}");
            assert_eq!(response.header("cache-control"), Some("no-store"));
            Poll::Token(token.to_owned())
        }
        400 => {
            assert!(body.get("access_token").is_none(), "{body}");
            Poll::Error(body["error"].as_str().expect("an error").to_owned())
        }
        status => panic!("a poll answered {status}: {body}"),
    }
}

/// Waits `seconds` and a little more, past a polling interval.
pub fn pause(seconds: u64) {
    thread::sleep(Duration::from_secs(seconds) + Duration::from_millis(300));
}

pub fn json(response: &Response) -> Value {
    serde_json::from_slice(&response.body).unwrap_or_else(|err| panic!("{err}: {}", text(response)))
}

pub fn text(response: &Response) -> String {
    format!(
        "{} {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    )
}
