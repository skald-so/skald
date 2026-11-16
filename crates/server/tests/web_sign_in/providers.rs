//! The providers' answer: which sign-in providers the server has settings for and whether its
//! sign-up is open, to anyone, without the lists of who may sign up.

use std::collections::BTreeSet;

use crate::common::device_flow::{json, text};
use crate::common::google::{Google, Setup};
use crate::common::sign_in::SignIn;
use crate::common::spec::{operation_violations, spec};
use crate::common::{Response, Server};

/// The sign-in providers the server has settings for and whether its sign-up is open: `GET`.
pub const PROVIDERS: &str = "/auth/providers";

/// `GET /auth/providers` straight to `server`, without a session, stating the protocol version;
/// checked against the spec.
fn providers_response(server: &Server) -> Response {
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let response = server.get_with(PROVIDERS, &[(skald_proto::VERSION_HEADER, &version)]);
    let violations = operation_violations(&spec(), "get", PROVIDERS, &response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
    response
}

/// The providers `server` names and its sign-up, `open` or `closed`; fails unless it answers `200`
/// with only those.
fn providers(server: &Server) -> (BTreeSet<String>, String) {
    let response = providers_response(server);
    assert_eq!(response.status, 200, "{}", text(&response));
    let body = json(&response);
    let fields: BTreeSet<&str> = body
        .as_object()
        .unwrap_or_else(|| panic!("an object: {body}"))
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(fields, BTreeSet::from(["providers", "sign_up"]), "{body}");
    let named: BTreeSet<String> = body["providers"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of providers: {body}"))
        .iter()
        .map(|provider| provider.as_str().expect("a provider").to_owned())
        .collect();
    let count = body["providers"].as_array().map_or(0, Vec::len);
    assert_eq!(named.len(), count, "each provider once: {body}");
    let sign_up = body["sign_up"].as_str().unwrap_or_else(|| panic!("{body}"));
    (named, sign_up.to_owned())
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn server_with_github_only_names_github() {
    let stand = SignIn::start();
    assert_eq!(
        providers(&stand.server),
        (set(&["github"]), "open".to_owned())
    );
}

#[test]
fn server_with_google_only_names_google() {
    let google = Google::start();
    assert_eq!(
        providers(&google.server),
        (set(&["google"]), "open".to_owned())
    );
}

#[test]
fn server_with_both_providers_names_both_and_open_sign_up() {
    for args in [&[][..], &["--sign-up", "open"][..]] {
        let both = Google::launch(Setup {
            github: true,
            args,
            ..Setup::default()
        });
        assert_eq!(
            providers(&both.server),
            (set(&["github", "google"]), "open".to_owned()),
            "{args:?}"
        );
    }
}

/// While sign-up is closed, the answer says so and names nothing of its lists.
#[test]
fn closed_sign_up_is_named_without_its_lists() {
    let listed = [
        "listed-login-standin",
        "listed.person@example.test",
        "listed-org.example.test",
    ];
    let closed = Google::launch(Setup {
        github: true,
        args: &[
            "--sign-up",
            "closed",
            "--sign-up-github-logins",
            listed[0],
            "--sign-up-google-emails",
            listed[1],
            "--sign-up-google-domains",
            listed[2],
        ],
        ..Setup::default()
    });
    assert_eq!(
        providers(&closed.server),
        (set(&["github", "google"]), "closed".to_owned())
    );
    let body = text(&providers_response(&closed.server));
    for entry in listed {
        let lower = entry.to_lowercase();
        assert!(!body.to_lowercase().contains(&lower), "{entry} in {body}");
    }
}
