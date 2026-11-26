use std::process::{Command, Stdio};
use std::time::Duration;

use skald_oidc_stand_in::{Key, Provider};

use super::common::browser::Browser;
use super::common::google::{Google, Setup, text};
use super::common::{SERVER, TestDb, wait};
use super::{CALLBACK, CLIENT_FLAG, CLIENT_ID, CLIENT_SECRET, ISSUER_FLAG, SECRET_ENV, START};
use super::{assert_providers, spec};

#[test]
fn openapi_has_corporate_browser_start_and_callback() {
    let spec = spec();
    for path in [START, CALLBACK] {
        assert!(
            spec["paths"][path]["get"].is_object(),
            "OpenAPI needs GET {path}"
        );
    }
}

#[test]
fn absent_corporate_settings_leave_existing_providers_available() {
    let site = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });
    site.server.assert_healthy();
    assert_providers(&site.server, &["github", "google"]);
    for path in [START, CALLBACK] {
        let response = site.server.get(path);
        assert!(
            (400..500).contains(&response.status),
            "GET {path}: {}",
            text(&response)
        );
        assert_eq!(response.header("location"), None);
    }

    let browser = Browser::launch();
    let person = site.person("existing-google-subject", "existing@example.test");
    assert_eq!(site.signed_up(&browser, &person).username, "existing");
}

#[track_caller]
fn rejects(db: &TestDb, args: &[&str], env: &[(&str, &str)], names: &[&str]) {
    let mut child = Command::new(SERVER)
        .args(["--listen", "127.0.0.1:0", "--database-url", &db.url()])
        .args(["--public-url", "http://127.0.0.1:1"])
        .env_remove(SECRET_ENV)
        .args(args)
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start skald-server");
    let status = wait(&mut child, Duration::from_secs(15));
    let output = child.wait_with_output().expect("read server output");
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !status.success(),
        "accepted incomplete or invalid corporate settings"
    );
    assert!(!printed.contains("unexpected argument"), "{printed}");
    assert!(
        !printed.contains(CLIENT_SECRET),
        "startup printed the client secret"
    );
    let diagnostic = printed.to_ascii_lowercase().replace('_', "-");
    for name in names {
        assert!(diagnostic.contains(name), "expected {name:?} in {printed}");
    }
}

#[test]
fn incomplete_corporate_settings_name_the_missing_value() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    let issuer = [ISSUER_FLAG, provider.issuer()];
    let client = [CLIENT_FLAG, CLIENT_ID];
    let secret = [(SECRET_ENV, CLIENT_SECRET)];
    rejects(&db, &client, &secret, &["corporate", "issuer"]);
    rejects(&db, &issuer, &secret, &["corporate", "client-id"]);
    rejects(
        &db,
        &[issuer, client].concat(),
        &[],
        &["corporate", "secret"],
    );
}

#[test]
fn invalid_corporate_issuer_is_refused() {
    let db = TestDb::create();
    let issuer = "http://workforce.example.test";
    rejects(
        &db,
        &[ISSUER_FLAG, issuer, CLIENT_FLAG, CLIENT_ID],
        &[(SECRET_ENV, CLIENT_SECRET)],
        &["issuer", "https"],
    );
}

#[test]
fn issuer_already_configured_for_google_is_refused() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    rejects(
        &db,
        &[
            ISSUER_FLAG,
            provider.issuer(),
            CLIENT_FLAG,
            CLIENT_ID,
            "--google-issuer",
            provider.issuer(),
            "--google-client-id",
            super::common::google::CLIENT_ID,
        ],
        &[
            (SECRET_ENV, CLIENT_SECRET),
            (
                super::common::google::SECRET_ENV,
                super::common::google::CLIENT_SECRET,
            ),
        ],
        &["issuer", "google", "corporate"],
    );
}
