//! The server signs in with the providers it has settings for, starts while the provider is
//! unreachable, and takes Google's issuer by default.

use std::process::Command;

use super::common::browser::Browser;
use super::common::spec::{spec, spec_violations};
use super::common::{Response, SERVER, Server, TestDb};
use super::{CALLBACK, GITHUB_START, Google, START, Setup, text};

/// Checks that `response` to `GET <path>` refuses a sign-in: an error that sends the browser
/// nowhere, as the OpenAPI spec lists it.
#[track_caller]
fn refuses(path: &str, response: &Response) {
    assert!(
        (400..600).contains(&response.status),
        "GET {path}: {} {}",
        response.status,
        text(response)
    );
    assert_eq!(response.header("location"), None, "GET {path}");
    assert_eq!(
        spec_violations(&spec(), path, response),
        Vec::<String>::new()
    );
}

#[test]
fn with_only_googles_settings_google_sign_in_works_and_githubs_is_refused() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238987000", "ada.only@example.com");
    google.signed_up(&browser, &ada);

    refuses(GITHUB_START, &google.server.get(GITHUB_START));
    let callback = "/auth/github/callback";
    refuses(
        callback,
        &google
            .server
            .get(&format!("{callback}?code=0123456789abcdef0123&state=abc")),
    );
}

#[test]
fn with_only_githubs_settings_google_sign_in_is_refused() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        github: true,
        no_google: true,
        ..Setup::default()
    });

    refuses(START, &google.server.get(START));
    refuses(
        CALLBACK,
        &google
            .server
            .get(&format!("{CALLBACK}?code=4%2F0Aabc&state=abc")),
    );

    // GitHub sign-in still works.
    let octo = google.github_user(7_301_553_001, "ada-onlygithub-standin", "ada@example.com");
    let page = browser.page();
    google.github_sign_in(&page, &octo);
    assert_eq!(
        google.account(&page).github_login.as_deref(),
        Some(octo.login.as_str())
    );
}

#[test]
fn server_starts_while_the_provider_is_down_and_google_sign_in_works_once_it_is_up() {
    let browser = Browser::launch();
    let mut google = Google::launch(Setup {
        down: true,
        ..Setup::default()
    });
    let ada = google.person("110248495921238987001", "ada.down@example.com");

    google.server.assert_healthy();

    // While the provider is down, a sign-in signs no one in, and the start says so as the spec
    // lists it.
    let start = google.server.get(START);
    assert_eq!(
        spec_violations(&spec(), START, &start),
        Vec::<String>::new()
    );
    let page = browser.page();
    page.goto(&google.start_url(None));
    google.assert_signed_out(&page);
    google.assert_no_account(&ada, "the provider is down");

    google.provider.restart();
    google.signed_up(&browser, &ada);
}

#[test]
fn without_any_settings_health_is_ok_and_google_sign_in_is_refused() {
    let db = TestDb::create();
    let server = Server::start(&db);
    server.assert_healthy();

    refuses(START, &server.get(START));
    refuses(
        CALLBACK,
        &server.get(&format!("{CALLBACK}?code=4%2F0Aabc&state=abc")),
    );
}

#[test]
fn issuer_defaults_to_googles() {
    let help = Command::new(SERVER)
        .arg("--help")
        .output()
        .expect("run skald-server --help");
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    // The setting's help: from its flag to the next flag's line.
    let at = help
        .find("--google-issuer")
        .unwrap_or_else(|| panic!("--help names --google-issuer: {help}"));
    let issuer = &help[at..];
    let issuer = &issuer[..issuer.find("\n  -").unwrap_or(issuer.len())];
    assert!(
        issuer.contains("[default: https://accounts.google.com]"),
        "{issuer}"
    );
    assert!(help.contains("--google-client-id"), "{help}");
}
