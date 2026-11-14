//! Acceptance tests of the check of a server's Google sign-in, run against a local `skald-server`
//! with Google's settings against the OIDC stand-in, which plays Google. A front before the server
//! and one before the stand-in change their answers as a broken or misconfigured server or Google
//! would. No test needs a real Google client or contacts Google or a real server.
//!
//! The check: `script/check-google-sign-in <server address> <client ID>`, with Google's issuer in
//! `GOOGLE_ISSUER`, `https://accounts.google.com` without it. It needs no person and runs these
//! steps in order, each only after the one before passed:
//! - `verification page`: the device flow's verification page on that server, without a session,
//!   offers Google sign-in, a link on that server whose text names Google; it sends the browser to
//!   the `authorization_endpoint` of the issuer's metadata (OpenID Connect Discovery 1.0, §4) with
//!   that client ID, the callback `<address>/auth/google/callback`, the scopes `openid` and `email`
//!   only, a `state`, a `nonce` and an S256 PKCE challenge;
//! - `authorization`: that authorization URL, followed, answers neither `redirect_uri_mismatch` nor
//!   `invalid_client`, directly or on a page it redirects to.
//!
//! Steps of its own before `verification page`, such as reading the metadata or a device
//! authorization, each print their own line too. It prints only a line `<step>: pass` or
//! `<step>: fail[: <reason>]` for each step it runs, then a last line `pass` or `fail`; each line
//! may start with `check-google-sign-in: `. It never prints a `state`, `nonce`, PKCE challenge,
//! user or device code the server gave. It exits 0 only when every step passes.

mod common;

use std::sync::Arc;

use skald_oidc_stand_in::{Client, Endpoint};
use url::Url;

use common::check::{
    Answer, ELSEWHERE, Front, Run, Tamper, answer_with, assert_no_codes, callbacks_off, on,
    run_script, set_param, to_provider, unchanged,
};
use common::google::{CALLBACK, CLIENT_ID, CLIENT_SECRET, Google, START, Setup, param, text};
use common::sign_in::{CALLBACK as GITHUB_CALLBACK, SignIn, is_s256_challenge};

/// The check.
const CHECK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../script/check-google-sign-in"
);
/// The prefix a line of the check's output may start with.
const PREFIX: &str = "check-google-sign-in: ";
/// The steps the tests name.
const PAGE_STEP: &str = "verification page";
const AUTHORIZATION_STEP: &str = "authorization";
/// The verification page.
const VERIFICATION_PAGE: &str = "/device";
/// The stand-in's authorization endpoint and metadata, under its issuer.
const AUTHORIZE: &str = "/authorize";
const METADATA: &str = "/.well-known/openid-configuration";
/// A client ID other than the server's.
const OTHER_CLIENT_ID: &str = "skald-stand-in-google-other-client";
/// The parameters of the authorization request the check holds back from its output.
const HELD_BACK: [&str; 3] = ["state", "nonce", "code_challenge"];

/// A line of the check's output.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A step and whether it passed.
    Step(String, bool),
    /// The check's result: whether it passed.
    Result(bool),
}

/// Whether `name` is a step's name: lowercase words.
fn is_step_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == '-')
        && !["pass", "fail"].contains(&name)
}

/// The output's lines, each read as a [`Line`]; fails on any other line.
fn lines(run: &Run) -> Vec<Line> {
    let read = |line: &str| {
        let line = line.strip_prefix(PREFIX).unwrap_or(line);
        match line {
            "pass" => return Some(Line::Result(true)),
            "fail" => return Some(Line::Result(false)),
            _ => {}
        }
        let (name, rest) = line.split_once(": ")?;
        if !is_step_name(name) {
            return None;
        }
        if rest == "pass" {
            Some(Line::Step(name.to_owned(), true))
        } else if rest == "fail" || rest.starts_with("fail: ") {
            Some(Line::Step(name.to_owned(), false))
        } else {
            None
        }
    };
    run.output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            read(line).unwrap_or_else(|| {
                panic!(
                    "{}: a line that is neither a step nor the result: {line:?}\n{}",
                    run.case, run.output
                )
            })
        })
        .collect()
}

/// Checks that the check passed: every step it ran, `verification page` and then `authorization`
/// among them, then the result.
fn assert_passes(run: &Run) {
    let lines = lines(run);
    let context = format!("{}: expected to pass\n{}", run.case, run.output);
    assert!(run.status.success(), "{context}");
    let (last, steps) = lines.split_last().expect(&context);
    assert_eq!(last, &Line::Result(true), "{context}");
    let at = |step: &str| {
        let at = steps.iter().position(|line| line == &pass(step));
        at.unwrap_or_else(|| panic!("{context}"))
    };
    assert!(at(PAGE_STEP) < at(AUTHORIZATION_STEP), "{context}");
    for line in steps {
        assert!(matches!(line, Line::Step(_, true)), "{context}");
    }
    let names: Vec<_> = steps.iter().filter_map(step_name).collect();
    let mut once = names.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), names.len(), "each step once: {context}");
}

/// Checks that the check failed at `failed`, after every step before it passed, and ran no step
/// after it.
fn assert_fails_at(run: &Run, failed: &str) {
    let lines = lines(run);
    let context = format!(
        "{}: expected to fail at {failed:?}\n{}",
        run.case, run.output
    );
    assert!(!run.status.success(), "{context}");
    let (last, steps) = lines.split_last().expect(&context);
    assert_eq!(last, &Line::Result(false), "{context}");
    let (fail, before) = steps.split_last().expect(&context);
    assert_eq!(fail, &Line::Step(failed.to_owned(), false), "{context}");
    for line in before {
        assert!(matches!(line, Line::Step(_, true)), "{context}");
        assert_ne!(step_name(line).as_deref(), Some(failed), "{context}");
    }
    if failed == AUTHORIZATION_STEP {
        assert!(before.contains(&pass(PAGE_STEP)), "{context}");
    } else {
        assert!(
            !before
                .iter()
                .any(|line| step_name(line).as_deref() == Some(AUTHORIZATION_STEP)),
            "{context}"
        );
    }
}

fn pass(step: &str) -> Line {
    Line::Step(step.to_owned(), true)
}

fn step_name(line: &Line) -> Option<String> {
    match line {
        Line::Step(name, _) => Some(name.clone()),
        Line::Result(_) => None,
    }
}

/// A server with Google's and, if asked, GitHub's settings, behind a front: its public address is
/// the front's. Its issuer is a front before the stand-in, whose metadata names the front instead of
/// the stand-in.
struct Stage {
    google: Google,
    front: Front,
    /// The front before the stand-in: the issuer the server and the check have.
    provider: Front,
}

impl Stage {
    /// A server with Google's and GitHub's settings, as staging has.
    fn start() -> Self {
        Self::launch(true)
    }

    fn launch(github: bool) -> Self {
        let mut front = Front::bind();
        let mut provider = Front::bind();
        let google = Google::launch(Setup {
            public: Some(&front.url),
            issuer: Some(&provider.url),
            github,
            ..Setup::default()
        });
        front.serve(google.server.base());
        provider.set_tamper(at_provider(&google, &provider, unchanged()));
        provider.serve(google.provider.issuer());
        Stage {
            google,
            front,
            provider,
        }
    }

    /// Makes the front before the stand-in change its answers by `tamper` from now on.
    fn at_provider(&self, tamper: Tamper) {
        let tamper = at_provider(&self.google, &self.provider, tamper);
        self.provider.set_tamper(tamper);
    }

    /// The server's address, as the check is given it.
    fn address(&self) -> &str {
        &self.front.url
    }

    /// Registers with the stand-in a client of `client_id` and `redirect_uri` instead of the
    /// server's.
    fn register(&self, client_id: &str, redirect_uri: &str) {
        self.google.provider.register(Client {
            client_id: client_id.to_owned(),
            client_secret: CLIENT_SECRET.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
        });
    }

    /// Registers the server's own client with the stand-in again.
    fn register_own(&self) {
        self.register(CLIENT_ID, &format!("{}{CALLBACK}", self.address()));
    }

    /// The codes the check must not print: the server's device and user codes, and the `state`,
    /// `nonce` and PKCE challenge of each sign-in it started.
    fn codes(&self) -> Vec<String> {
        let mut codes = self.front.codes();
        for exchange in self.front.exchanges() {
            if exchange.method != "GET" || exchange.path != START {
                continue;
            }
            let Some(location) = exchange.answer.header("location") else {
                continue;
            };
            for name in HELD_BACK {
                codes.extend(param(location, name).filter(|value| value.len() >= 8));
            }
        }
        codes
    }

    /// Runs the check of this server expecting `client_id`, with the issuer `issuer`, the server's
    /// answers changed by `tamper`; checks that it printed no code the server gave.
    fn check_with(&self, case: &str, client_id: &str, issuer: &str, tamper: Tamper) -> Run {
        self.front.set_tamper(tamper);
        let run = run_script(
            case,
            CHECK,
            &[self.address(), client_id],
            &[("GOOGLE_ISSUER", issuer)],
        );
        self.front.assert_tamper_held();
        self.provider.assert_tamper_held();
        assert_no_codes(&run, &self.codes());
        run
    }

    fn check(&self, case: &str, client_id: &str, tamper: Tamper) -> Run {
        self.check_with(case, client_id, &self.provider.url, tamper)
    }

    /// Runs the check of this server, its answers changed by `tamper`, and checks that it fails at
    /// `step`.
    fn assert_fails_at(&self, step: &str, case: &str, tamper: Tamper) {
        assert_fails_at(&self.check(case, CLIENT_ID, tamper), step);
    }
}

/// The tamper of `provider`, the front before `google`'s stand-in: the metadata names the front
/// instead of the stand-in, and `tamper` changes the answers after that.
fn at_provider(google: &Google, provider: &Front, tamper: Tamper) -> Tamper {
    let (stand_in, front) = (google.provider.issuer().to_owned(), provider.url.clone());
    Arc::new(move |method, path, mut answer| {
        if method == "GET" && path == METADATA {
            let body = text(&answer).replace(&stand_in, &front);
            answer.body = body.into_bytes();
        }
        tamper(method, path, answer)
    })
}

/// A tamper that changes the address the Google sign-in start sends the browser to, by `change`.
fn to_google(change: impl Fn(&mut Url) + Send + Sync + 'static) -> Tamper {
    to_provider(START, change)
}

/// A tamper that sets the authorization request's parameter `name` to `value`, or removes it.
fn to_google_with(name: &'static str, value: Option<&'static str>) -> Tamper {
    to_google(move |url| set_param(url, name, value))
}

/// A tamper that changes the verification page's body by `change`.
fn page(change: impl Fn(String) -> String + Send + Sync + 'static) -> Tamper {
    on("GET", VERIFICATION_PAGE, move |answer| {
        let body = change(text(&answer));
        Answer {
            body: body.into_bytes(),
            ..answer
        }
    })
}

/// `body` without its links, forms and buttons that name Google.
fn without_google(body: &str) -> String {
    let mut body = body.to_owned();
    for (open, close) in [
        ("<a ", "</a>"),
        ("<form", "</form>"),
        ("<button", "</button>"),
    ] {
        let mut kept = String::new();
        let mut rest = body.as_str();
        while let Some(at) = rest.find(open) {
            let Some(length) = rest[at..].find(close) else {
                break;
            };
            let end = at + length + close.len();
            kept.push_str(&rest[..at]);
            if !rest[at..end].contains("Google") {
                kept.push_str(&rest[at..end]);
            }
            rest = &rest[end..];
        }
        kept.push_str(rest);
        body = kept;
    }
    body
}

/// An HTML page of `status` that says `text`.
fn html(status: u16, text: &str) -> Answer {
    Answer::made(status, "text/html; charset=utf-8", text)
}

/// A redirect to `location`.
fn redirect(location: &str) -> Answer {
    let mut answer = Answer::made(302, "text/html", "");
    answer.set_header("location", location);
    answer
}

#[test]
fn check_passes_against_a_server_with_google_sign_in() {
    let stage = Stage::start();
    let asked = stage.google.provider.requests_to(Endpoint::Authorize).len();

    assert_passes(&stage.check("a server with Google sign-in", CLIENT_ID, unchanged()));

    let requests = stage.google.provider.requests_to(Endpoint::Authorize);
    assert_eq!(requests.len(), asked + 1, "the check followed the URL");
    let request = &requests[asked].params;
    assert_eq!(request["client_id"], CLIENT_ID);
    assert_eq!(
        request["redirect_uri"],
        format!("{}{CALLBACK}", stage.address())
    );
    assert!(
        stage.codes().len() >= HELD_BACK.len(),
        "the check started a Google sign-in on the server: {:?}",
        stage.codes()
    );
    assert!(stage.google.provider.codes().is_empty(), "no one signed in");
}

#[test]
fn check_passes_against_a_server_with_only_google_sign_in() {
    let stage = Stage::launch(false);

    assert_passes(&stage.check("a server with Google sign-in only", CLIENT_ID, unchanged()));
}

#[test]
fn check_passes_when_the_provider_redirects_to_its_sign_in_page() {
    let stage = Stage::start();
    let to_sign_in = redirect(&format!("{}/signin/identifier", stage.provider.url));
    stage.at_provider(Arc::new(move |_, path, answer| match path {
        AUTHORIZE => to_sign_in.clone(),
        "/signin/identifier" => html(200, "<h1>Sign in with Google</h1>"),
        _ => answer,
    }));

    assert_passes(&stage.check("the provider asks to sign in", CLIENT_ID, unchanged()));
}

#[test]
fn check_fails_at_the_verification_page_when_it_offers_no_google_sign_in() {
    let stage = Stage::start();
    let cases = [
        (
            "the page fails",
            answer_with(
                "GET",
                VERIFICATION_PAGE,
                html(500, "<p>The server failed.</p>"),
            ),
        ),
        (
            "the page has no way to sign in",
            page(|_| "<h1>Sign in a machine</h1><p>Come back later.</p>".to_owned()),
        ),
        (
            "the page offers GitHub sign-in only",
            page(|body| {
                assert!(body.contains("Google"), "a Google sign-in: {body}");
                let body = without_google(&body);
                assert!(!body.contains("/auth/google"), "no Google sign-in: {body}");
                assert!(body.contains("GitHub"), "a GitHub sign-in: {body}");
                body
            }),
        ),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(PAGE_STEP, case, tamper);
    }
}

#[test]
fn check_fails_at_the_verification_page_of_a_server_without_google_settings() {
    let mut front = Front::bind();
    let stand = SignIn::behind(&front.url, &[]);
    front.serve(stand.server.base());
    let google = Google::start();

    let run = run_script(
        "a server with GitHub sign-in only",
        CHECK,
        &[&front.url, CLIENT_ID],
        &[("GOOGLE_ISSUER", google.provider.issuer())],
    );

    assert_fails_at(&run, PAGE_STEP);
    assert_no_codes(&run, &front.codes());
}

#[test]
fn check_fails_at_the_verification_page_when_it_sends_the_browser_elsewhere() {
    let stage = Stage::start();
    let front = stage.address().to_owned();
    let cases = [
        (
            "the page's Google sign-in leads to another server",
            page(move |body| {
                let elsewhere = format!("\"http://{ELSEWHERE}/auth/google");
                let changed = body
                    .replace("\"/auth/google", &elsewhere)
                    .replace(&format!("\"{front}/auth/google"), &elsewhere);
                assert_ne!(changed, body, "a Google sign-in on the server: {body}");
                changed
            }),
        ),
        (
            "the sign-in start fails",
            answer_with("GET", START, html(500, "<p>The server failed.</p>")),
        ),
        (
            "the sign-in start answers a page",
            answer_with("GET", START, html(200, "<p>Sign in with Google</p>")),
        ),
        (
            "the sign-in start sends the browser to another host",
            to_google(|url| url.set_host(Some(ELSEWHERE)).expect("a host")),
        ),
        (
            "the sign-in start sends the browser to another page of the provider",
            to_google(|url| url.set_path("/o/oauth2/v2/auth")),
        ),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(PAGE_STEP, case, tamper);
    }

    // The server's provider is not the issuer the check has: the stand-in itself, whose metadata
    // names its own authorization endpoint, not the front's the server sends the browser to.
    let run = stage.check_with(
        "the server's provider is another issuer",
        CLIENT_ID,
        stage.google.provider.issuer(),
        unchanged(),
    );
    assert_fails_at(&run, PAGE_STEP);
}

#[test]
fn check_fails_at_the_verification_page_with_another_client_id() {
    let stage = Stage::start();

    let longer = format!("{CLIENT_ID}0");
    let part = &CLIENT_ID[..CLIENT_ID.len() - 3];
    for (case, expected) in [
        ("another client ID expected", OTHER_CLIENT_ID),
        ("a longer client ID expected", longer.as_str()),
        ("a part of the client ID expected", part),
    ] {
        assert_ne!(expected, CLIENT_ID);
        assert_fails_at(&stage.check(case, expected, unchanged()), PAGE_STEP);
    }

    let cases = [
        (
            "the server sends another client ID",
            to_google_with("client_id", Some(OTHER_CLIENT_ID)),
        ),
        (
            "the server sends no client ID",
            to_google_with("client_id", None),
        ),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(PAGE_STEP, case, tamper);
    }
}

#[test]
fn check_fails_at_the_verification_page_with_another_callback() {
    let stage = Stage::start();
    let address = stage.address();
    let mut callbacks = callbacks_off(address, CALLBACK);
    callbacks.push(("GitHub's callback", format!("{address}{GITHUB_CALLBACK}")));
    callbacks.push((
        "a callback under Google's",
        format!("{address}{CALLBACK}/other"),
    ));
    for (case, callback) in callbacks {
        let tamper = to_google(move |url| set_param(url, "redirect_uri", Some(&callback)));
        stage.assert_fails_at(PAGE_STEP, case, tamper);
    }
    stage.assert_fails_at(
        PAGE_STEP,
        "no callback",
        to_google_with("redirect_uri", None),
    );
}

#[test]
fn check_fails_at_the_verification_page_with_other_scopes() {
    let stage = Stage::start();
    let cases = [
        ("openid only", Some("openid")),
        ("email only", Some("email")),
        ("a scope more", Some("openid email profile")),
        ("other scopes", Some("openid profile")),
        ("no scope", None),
    ];
    for (case, scope) in cases {
        stage.assert_fails_at(PAGE_STEP, case, to_google_with("scope", scope));
    }
}

#[test]
fn check_fails_at_the_verification_page_without_state_nonce_or_pkce_challenge() {
    let stage = Stage::start();
    let mut cases = vec![
        ("no state", to_google_with("state", None)),
        ("an empty state", to_google_with("state", Some(""))),
        ("no nonce", to_google_with("nonce", None)),
        ("an empty nonce", to_google_with("nonce", Some(""))),
        ("no PKCE challenge", to_google_with("code_challenge", None)),
        (
            "no PKCE challenge method",
            to_google_with("code_challenge_method", None),
        ),
        (
            "a plain PKCE challenge",
            to_google_with("code_challenge_method", Some("plain")),
        ),
        (
            "a PKCE challenge too short for S256",
            to_google_with("code_challenge", Some("c2hvcnQ")),
        ),
    ];
    let bad = "not+a/base64url=challenge-of-43-characters=";
    assert_eq!(bad.len(), 43);
    assert!(!is_s256_challenge(bad));
    cases.push((
        "a PKCE challenge that is not base64url",
        to_google_with("code_challenge", Some(bad)),
    ));
    cases.push((
        "no PKCE challenge or method",
        to_google(|url| {
            set_param(url, "code_challenge", None);
            set_param(url, "code_challenge_method", None);
        }),
    ));
    for (case, tamper) in cases {
        stage.assert_fails_at(PAGE_STEP, case, tamper);
    }
}

#[test]
fn check_fails_at_authorization_when_the_provider_answers_redirect_uri_mismatch() {
    let stage = Stage::start();
    let callback = format!("{}{CALLBACK}", stage.address());
    for (case, registered) in [
        (
            "the client's redirect URI is on another host",
            format!("http://{ELSEWHERE}{CALLBACK}"),
        ),
        (
            "the client's redirect URI ends in a slash",
            format!("{callback}/"),
        ),
    ] {
        stage.register(CLIENT_ID, &registered);
        let run = stage.check(case, CLIENT_ID, unchanged());
        let requests = stage.google.provider.requests_to(Endpoint::Authorize);
        assert_eq!(
            requests
                .last()
                .map(|request| request.params["redirect_uri"].clone()),
            Some(callback.clone()),
            "{case}: the stand-in got the server's callback"
        );
        assert_fails_at(&run, AUTHORIZATION_STEP);
    }
    stage.register_own();
    assert_passes(&stage.check("the client registered again", CLIENT_ID, unchanged()));
}

#[test]
fn check_fails_at_authorization_when_the_provider_answers_invalid_client() {
    let stage = Stage::start();
    let callback = format!("{}{CALLBACK}", stage.address());

    stage.register(OTHER_CLIENT_ID, &callback);
    assert_fails_at(
        &stage.check("the provider has no such client", CLIENT_ID, unchanged()),
        AUTHORIZATION_STEP,
    );
}

#[test]
fn check_fails_at_authorization_when_the_provider_redirects_to_its_error_page() {
    let stage = Stage::start();
    let error_page = format!("{}/signin/oauth/error", stage.provider.url);
    let to_error = redirect(&format!("{error_page}?authError=Cg9zdGFuZC1pbg"));
    // The page names the error whatever its status: a check that reads only the status misses it.
    for (error, status) in [
        ("redirect_uri_mismatch", 400),
        ("invalid_client", 401),
        ("redirect_uri_mismatch", 200),
        ("invalid_client", 200),
    ] {
        let to_error = to_error.clone();
        let says = html(
            status,
            &format!("<h1>Access blocked</h1><p>Error {status}: {error}</p>"),
        );
        stage.at_provider(Arc::new(move |_, path, answer| match path {
            AUTHORIZE => to_error.clone(),
            "/signin/oauth/error" => says.clone(),
            _ => answer,
        }));

        let run = stage.check(
            &format!("the provider's error page of status {status} says {error}"),
            CLIENT_ID,
            unchanged(),
        );
        assert_fails_at(&run, AUTHORIZATION_STEP);
    }
}

#[test]
fn check_prints_only_its_steps_and_no_code() {
    let stage = Stage::start();
    let runs = [
        stage.check("a server with Google sign-in", CLIENT_ID, unchanged()),
        stage.check("no nonce", CLIENT_ID, to_google_with("nonce", None)),
        stage.check(
            "the sign-in start sends the browser to another host",
            CLIENT_ID,
            to_google(|url| url.set_host(Some(ELSEWHERE)).expect("a host")),
        ),
    ];

    let codes = stage.codes();
    assert!(
        codes.len() >= runs.len() * HELD_BACK.len(),
        "a state, a nonce and a PKCE challenge per run: {codes:?}"
    );
    for run in &runs {
        assert_no_codes(run, &codes);
        // Every line is a step or the result.
        let _: Vec<Line> = lines(run);
    }
    assert_passes(&runs[0]);
    assert_fails_at(&runs[1], PAGE_STEP);
    assert_fails_at(&runs[2], PAGE_STEP);
}
