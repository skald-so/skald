//! The check of a server's GitHub sign-in, run against a local `skald-server` set up for GitHub
//! sign-in with the GitHub stand-in, behind a front that can change the server's answers as a broken
//! or misconfigured server would. No test needs a real OAuth App or contacts GitHub or a real server.
//!
//! The check: `script/check-github-sign-in <server address> <client ID>`, with GitHub's address in
//! `GITHUB_URL`, `https://github.com` without it. It needs no person and runs these steps in order:
//! - `device authorization`: `POST <address>/auth/device` gives a user code, a device code and a
//!   verification page address on that server;
//! - `verification page`: the page sends a browser without a session to GitHub's authorization page,
//!   `<GitHub>/login/oauth/authorize`, with that client ID and a callback on that server;
//! - `polling`: `POST <address>/auth/token` with the new device code answers
//!   `authorization_pending`.
//!
//! It prints only a line `<step>: pass` or `<step>: fail[: <reason>]` for each step it runs, then
//! a last line `pass` or `fail`; each line may start with `check-github-sign-in: `. It never prints
//! a user or device code. It exits 0 only when every step passes.

mod common;

use std::net::TcpListener;

use serde_json::{Value, json};
use skald_github_stand_in::{App, GitHub};
use url::Url;

use common::check::{
    Answer, ELSEWHERE, Front, Run, Tamper, answer_with, assert_no_codes, callbacks_off, on,
    run_script, set_param, to_provider, unchanged,
};
use common::device_flow::{DEVICE_AUTHORIZATION, INTERVAL, TOKEN, json as json_of};
use common::sign_in::{CALLBACK, CLIENT_ID, CLIENT_SECRET, START, SignIn};
use common::{Server, TestDb};

/// The check.
const CHECK: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../script/check-github-sign-in"
);
/// The prefix a line of the check's output may start with.
const PREFIX: &str = "check-github-sign-in: ";
/// The verification page.
const VERIFICATION_PAGE: &str = "/device";
/// A step of the check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    DeviceAuthorization,
    VerificationPage,
    Polling,
}

impl Step {
    const ALL: [Step; 3] = [
        Step::DeviceAuthorization,
        Step::VerificationPage,
        Step::Polling,
    ];

    fn name(self) -> &'static str {
        match self {
            Step::DeviceAuthorization => "device authorization",
            Step::VerificationPage => "verification page",
            Step::Polling => "polling",
        }
    }
}

/// A line of the check's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    /// A step and whether it passed.
    Step(Step, bool),
    /// The check's result: whether it passed.
    Result(bool),
}

impl Run {
    /// The output's lines, each read as a [`Line`]; fails on any other line.
    fn lines(&self) -> Vec<Line> {
        self.output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let line = line.strip_prefix(PREFIX).unwrap_or(line);
                let read = match line {
                    "pass" => Some(Line::Result(true)),
                    "fail" => Some(Line::Result(false)),
                    _ => Step::ALL.into_iter().find_map(|step| {
                        let rest = line.strip_prefix(step.name())?.strip_prefix(": ")?;
                        if rest == "pass" {
                            Some(Line::Step(step, true))
                        } else if rest == "fail" || rest.starts_with("fail: ") {
                            Some(Line::Step(step, false))
                        } else {
                            None
                        }
                    }),
                };
                read.unwrap_or_else(|| {
                    panic!(
                        "{}: a line that is neither a step nor the result: {line:?}\n{}",
                        self.case, self.output
                    )
                })
            })
            .collect()
    }

    /// Checks that the check passed: every step in order, then the result.
    fn assert_passes(&self) {
        let mut expected: Vec<Line> = Step::ALL.map(|step| Line::Step(step, true)).to_vec();
        expected.push(Line::Result(true));
        assert_eq!(self.lines(), expected, "{}:\n{}", self.case, self.output);
        assert!(
            self.status.success(),
            "{}: exit status {}\n{}",
            self.case,
            self.status,
            self.output
        );
    }

    /// Checks that the check failed at `failed`, after every step before it passed, and named no
    /// other step as failed.
    fn assert_fails_at(&self, failed: Step) {
        let lines = self.lines();
        let context = format!(
            "{}: expected to fail at {:?}\n{}",
            self.case, failed, self.output
        );
        assert!(!self.status.success(), "{context}");
        assert_eq!(lines.last(), Some(&Line::Result(false)), "{context}");
        assert!(!lines.contains(&Line::Result(true)), "{context}");
        for step in Step::ALL {
            let fails = lines
                .iter()
                .filter(|&&line| line == Line::Step(step, false));
            let passes = lines.iter().filter(|&&line| line == Line::Step(step, true));
            let (fails, passes) = (fails.count(), passes.count());
            if step == failed {
                assert_eq!((fails, passes), (1, 0), "{context}");
            } else if (step as usize) < (failed as usize) {
                assert_eq!((fails, passes), (0, 1), "{context}");
            } else {
                assert_eq!(fails, 0, "{context}");
            }
        }
    }
}

/// Runs the check against `address` with the expected client ID `client_id` and GitHub at `github`.
fn run_check(case: &str, address: &str, client_id: &str, github: &str) -> Run {
    run_script(
        case,
        CHECK,
        &[address, client_id],
        &[("GITHUB_URL", github)],
    )
}

/// A tamper that changes the address the sign-in start sends the browser to, by `change`.
fn to_github(change: impl Fn(&mut Url) + Send + Sync + 'static) -> Tamper {
    to_provider(START, change)
}

/// A server with GitHub sign-in through the stand-in, its codes polled every second, behind a
/// front: its public address is the front's.
struct Stage {
    stand: SignIn,
    front: Front,
}

impl Stage {
    fn start() -> Self {
        let mut front = Front::bind();
        let stand = SignIn::behind(&front.url, &[INTERVAL, "1"]);
        front.serve(stand.server.base());
        Stage { stand, front }
    }

    /// The server's address, as the check is given it.
    fn address(&self) -> &str {
        &self.front.url
    }

    /// Runs the check of this server expecting `client_id`, the server's answers changed by
    /// `tamper`; checks that it printed no code the server gave.
    fn check(&self, case: &str, client_id: &str, tamper: Tamper) -> Run {
        self.front.set_tamper(tamper);
        let run = run_check(case, self.address(), client_id, self.stand.github.url());
        self.front.assert_tamper_held();
        assert_no_codes(&run, &self.front.codes());
        run
    }

    /// Runs the check of this server, its answers changed by `tamper`, and checks that it fails at
    /// `step`.
    fn assert_fails_at(&self, step: Step, case: &str, tamper: Tamper) {
        self.check(case, CLIENT_ID, tamper).assert_fails_at(step);
    }
}

#[test]
fn check_passes_against_a_server_with_github_sign_in() {
    let stage = Stage::start();

    stage
        .check("a server with GitHub sign-in", CLIENT_ID, unchanged())
        .assert_passes();

    assert!(
        stage.front.codes().len() >= 2,
        "the check asked the server for a device code: {:?}",
        stage.front.codes()
    );
}

#[test]
fn check_fails_at_device_authorization_without_a_server_with_github_sign_in() {
    let github = GitHub::start(App {
        client_id: CLIENT_ID.to_owned(),
        client_secret: CLIENT_SECRET.to_owned(),
        callback: None,
    });

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let nothing = format!("http://{}", listener.local_addr().expect("local address"));
    drop(listener);
    run_check("nothing listens", &nothing, CLIENT_ID, github.url())
        .assert_fails_at(Step::DeviceAuthorization);

    let db = TestDb::create();
    let server = Server::start(&db);
    run_check(
        "a server without the GitHub settings",
        server.base(),
        CLIENT_ID,
        github.url(),
    )
    .assert_fails_at(Step::DeviceAuthorization);
}

#[test]
fn check_fails_at_device_authorization_when_it_gives_no_codes_or_page_on_that_server() {
    let stage = Stage::start();
    let post = |change: fn(&mut Value)| {
        on("POST", DEVICE_AUTHORIZATION, move |answer| {
            let mut grant = json_of(&answer);
            change(&mut grant);
            Answer::of_json(answer.status, &grant)
        })
    };
    let elsewhere = |grant: &mut Value| {
        let code = grant["user_code"].as_str().expect("a user code").to_owned();
        let page = format!("http://{ELSEWHERE}{VERIFICATION_PAGE}");
        grant["verification_uri_complete"] = json!(format!("{page}?user_code={code}"));
        grant["verification_uri"] = json!(page);
    };
    let cases = [
        (
            "the server fails",
            answer_with("POST", DEVICE_AUTHORIZATION, Answer::problem(500)),
        ),
        (
            "the server refuses",
            answer_with(
                "POST",
                DEVICE_AUTHORIZATION,
                Answer::of_json(400, &json!({ "error": "invalid_client" })),
            ),
        ),
        (
            "an answer that is not JSON",
            answer_with(
                "POST",
                DEVICE_AUTHORIZATION,
                Answer::made(200, "text/html", "<p>Hello</p>"),
            ),
        ),
        (
            "no user code",
            post(|grant| {
                grant
                    .as_object_mut()
                    .expect("an object")
                    .remove("user_code");
            }),
        ),
        (
            "no device code",
            post(|grant| {
                grant
                    .as_object_mut()
                    .expect("an object")
                    .remove("device_code");
            }),
        ),
        (
            "no verification page",
            post(|grant| {
                let grant = grant.as_object_mut().expect("an object");
                grant.remove("verification_uri");
                grant.remove("verification_uri_complete");
            }),
        ),
        ("a verification page on another server", post(elsewhere)),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(Step::DeviceAuthorization, case, tamper);
    }
}

#[test]
fn check_fails_at_the_verification_page_when_it_sends_the_browser_elsewhere() {
    let stage = Stage::start();
    let page = |change: fn(String) -> String| {
        on("GET", VERIFICATION_PAGE, move |answer| {
            let body = change(String::from_utf8_lossy(&answer.body).into_owned());
            Answer {
                body: body.into_bytes(),
                ..answer
            }
        })
    };
    let cases = [
        (
            "the page fails",
            answer_with(
                "GET",
                VERIFICATION_PAGE,
                Answer::made(500, "text/html", "<p>The server failed.</p>"),
            ),
        ),
        (
            "the page has no way to sign in",
            page(|_| "<h1>Sign in a machine</h1><p>Come back later.</p>".to_owned()),
        ),
        (
            "the page's sign-in leads to another server",
            page(|body| {
                assert!(
                    body.contains("href=\"/auth/github"),
                    "a sign-in link: {body}"
                );
                body.replace(
                    "href=\"/auth/github",
                    &format!("href=\"http://{ELSEWHERE}/auth/github"),
                )
            }),
        ),
        (
            "the sign-in start fails",
            answer_with(
                "GET",
                START,
                Answer::made(500, "text/html", "<p>The server failed.</p>"),
            ),
        ),
        (
            "the sign-in start sends the browser to another host",
            to_github(|url| url.set_host(Some(ELSEWHERE)).expect("a host")),
        ),
        (
            "the sign-in start sends the browser to another page of GitHub",
            to_github(|url| url.set_path("/login")),
        ),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(Step::VerificationPage, case, tamper);
    }
}

#[test]
fn check_fails_at_the_verification_page_with_another_client_id() {
    let stage = Stage::start();

    for (case, expected) in [
        ("another client ID expected", "Ov23li0another0client"),
        ("a longer client ID expected", "Ov23li0standin0client0"),
        ("a part of the client ID expected", "Ov23li0standin0"),
    ] {
        assert_ne!(expected, CLIENT_ID);
        stage
            .check(case, expected, unchanged())
            .assert_fails_at(Step::VerificationPage);
    }

    let cases = [
        (
            "the server sends another client ID",
            to_github(|url| set_param(url, "client_id", Some("Ov23li0another0client"))),
        ),
        (
            "the server sends no client ID",
            to_github(|url| set_param(url, "client_id", None)),
        ),
    ];
    for (case, tamper) in cases {
        stage.assert_fails_at(Step::VerificationPage, case, tamper);
    }
}

#[test]
fn check_fails_at_the_verification_page_with_a_callback_off_that_server() {
    let stage = Stage::start();
    let callbacks = callbacks_off(stage.address(), CALLBACK);
    for (case, callback) in callbacks {
        let tamper = to_github(move |url| set_param(url, "redirect_uri", Some(&callback)));
        stage.assert_fails_at(Step::VerificationPage, case, tamper);
    }
    stage.assert_fails_at(
        Step::VerificationPage,
        "no callback",
        to_github(|url| set_param(url, "redirect_uri", None)),
    );
}

#[test]
fn check_fails_at_polling_when_it_answers_anything_but_pending() {
    let stage = Stage::start();
    let error = |error: &str| Answer::of_json(400, &json!({ "error": error }));
    let cases = [
        ("slow down", error("slow_down")),
        ("denied", error("access_denied")),
        ("expired", error("expired_token")),
        ("an unknown device code", error("invalid_grant")),
        (
            "a token",
            Answer::of_json(
                200,
                &json!({ "access_token": "a-session-token", "token_type": "Bearer" }),
            ),
        ),
        ("the server fails", Answer::problem(500)),
        (
            "an answer that is not JSON",
            Answer::made(400, "text/html", "<p>authorization_pending</p>"),
        ),
    ];
    for (case, answer) in cases {
        stage.assert_fails_at(Step::Polling, case, answer_with("POST", TOKEN, answer));
    }
}

#[test]
fn check_prints_no_user_or_device_code() {
    let stage = Stage::start();

    let mut runs = vec![stage.check("a server with GitHub sign-in", CLIENT_ID, unchanged())];
    runs.push(stage.check(
        "polling answers denied",
        CLIENT_ID,
        answer_with(
            "POST",
            TOKEN,
            Answer::of_json(400, &json!({ "error": "access_denied" })),
        ),
    ));
    runs.push(stage.check(
        "the sign-in start sends the browser to another host",
        CLIENT_ID,
        to_github(|url| url.set_host(Some(ELSEWHERE)).expect("a host")),
    ));

    let codes = stage.front.codes();
    assert!(
        codes.len() >= 2 * runs.len(),
        "a device and a user code per run: {codes:?}"
    );
    for run in &runs {
        assert_no_codes(run, &codes);
    }
    runs[0].assert_passes();
    runs[1].assert_fails_at(Step::Polling);
    runs[2].assert_fails_at(Step::VerificationPage);
}
