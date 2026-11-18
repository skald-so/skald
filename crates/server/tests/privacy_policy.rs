//! The server's privacy policy: an address its operator sets, which the answer on sign-in gives and
//! the server's pages that start a sign-in link. Responses are checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The setting `--privacy-policy-url <URL>`, or `SKALD_PRIVACY_POLICY_URL`, is unset by default.
//! Only an absolute `https` URL; any other value stops the server at start, naming the setting.
//!
//! The API, as these tests use it, without a session:
//! - `GET /auth/providers`: `"privacy_policy_url": <URL>` with the setting, no such field without;
//! - `GET /device` and `GET /sign-in-methods`: with the setting, a link `Privacy policy` to it.

mod common;

use common::device_flow::{json, text};
use common::sign_in::{CLIENT_SECRET, SignIn};
use common::spec::{operation_violations, spec};
use common::{Server, TestDb};

/// The setting, as an argument and as an environment variable.
const SETTING: &str = "--privacy-policy-url";
const SETTING_ENV: &str = "SKALD_PRIVACY_POLICY_URL";

/// A policy's address with a query, whose `&` a page escapes.
const POLICY: &str = "https://policies.example/skald/privacy?lang=en&v=2";
/// The same address in an HTML attribute.
const POLICY_IN_HTML: &str = "https://policies.example/skald/privacy?lang=en&amp;v=2";

/// The server's pages that start a sign-in.
const PAGES: [&str; 2] = ["/device", "/sign-in-methods"];

/// A PostgreSQL URL where nothing listens: a server whose settings are taken exits at once too,
/// but with another error.
const CLOSED_DATABASE: &str = "postgres://skald@127.0.0.1:1/skald";

/// The `privacy_policy_url` of `GET /auth/providers` at `server`, checked against the spec; `None`
/// without one.
fn policy_of(server: &Server) -> Option<String> {
    let path = "/auth/providers";
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let response = server.get_with(path, &[(skald_proto::VERSION_HEADER, &version)]);
    let violations = operation_violations(&spec(), "get", path, &response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    body.get("privacy_policy_url").map(|url| {
        url.as_str()
            .unwrap_or_else(|| panic!("a string: {body}"))
            .to_owned()
    })
}

/// The page at `path` of `stand`'s server, without a session; fails unless it answers `200`.
fn page(stand: &SignIn, path: &str) -> String {
    let response = stand.server.get(path);
    assert_eq!(response.status, 200, "{path}: {}", text(&response));
    text(&response)
}

/// The answer gives the address set as an argument or in the environment, also on a server without
/// sign-in; without the setting, it has none.
#[test]
fn answer_gives_the_set_address_and_none_without_it() {
    let db = TestDb::create();
    let server = Server::launch(&db, "127.0.0.1:0", &[SETTING, POLICY], &[]);
    assert_eq!(policy_of(&server).as_deref(), Some(POLICY));

    let from_env = SignIn::on_with(TestDb::create(), &[], &[(SETTING_ENV, POLICY)]);
    assert_eq!(policy_of(&from_env.server).as_deref(), Some(POLICY));

    let without = SignIn::start();
    assert_eq!(policy_of(&without.server), None);
}

/// With the setting, the verification page and the page of the account's sign-in methods link the
/// policy; without it, neither does.
#[test]
fn pages_that_start_a_sign_in_link_the_policy_only_with_the_setting() {
    let with = SignIn::start_with(&[SETTING, POLICY], CLIENT_SECRET);
    let without = SignIn::start();
    for path in PAGES {
        let shown = page(&with, path);
        let link = format!("<a href=\"{POLICY_IN_HTML}\">Privacy policy</a>");
        assert!(shown.contains(&link), "{path}: {link} in {shown}");

        let shown = page(&without, path);
        assert!(!shown.contains("Privacy policy"), "{path}: {shown}");
        assert!(!shown.contains("policies.example"), "{path}: {shown}");
    }
}

/// What a server with the setting `value`, as an argument or from the environment if `env`, prints
/// on a database it cannot reach, where it stops either way.
fn start_output(value: &str, env: bool) -> String {
    let mut args = vec!["--listen", "127.0.0.1:0", "--database-url", CLOSED_DATABASE];
    let mut envs = vec![];
    if env {
        envs.push((SETTING_ENV, value));
    } else {
        args.extend([SETTING, value]);
    }
    let (started, output) = common::run(&args, &envs);
    assert!(!started, "{value:?}: {output}");
    output
}

/// A value other than an absolute https URL stops the server at start, as an argument or from the
/// environment, with an error naming the setting; an https URL does not.
#[test]
fn non_https_value_stops_the_server_at_start() {
    for env in [false, true] {
        let valid = start_output(POLICY, env);
        assert!(!valid.contains(SETTING), "env {env}: {valid}");
        assert!(valid.contains("PostgreSQL"), "env {env}: {valid}");
        for value in [
            "http://policies.example/privacy",
            "ftp://policies.example/privacy",
            "policies.example/privacy",
            "/privacy",
            "https://",
            "privacy policy",
        ] {
            let output = start_output(value, env);
            assert!(output.contains(SETTING), "{value:?}, env {env}: {output}");
            assert!(
                !output.contains("PostgreSQL"),
                "{value:?}, env {env}: {output}"
            );
        }
    }
}
