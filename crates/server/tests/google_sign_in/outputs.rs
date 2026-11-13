//! What the server keeps and prints: the database holds no email, Google token or client secret,
//! and the server's output never shows the client secret, an email, an ID token or an
//! authorization code.

use skald_oidc_stand_in::{Fault, Person};

use super::common::browser::Browser;
use super::common::{TestDb, assert_holds_none, run};
use super::{CLIENT_ID, CLIENT_SECRET, Google, SECRET_ENV, Setup, WRONG_SECRET};

/// What no database row and no output may show: the client secret, the people's emails and what
/// the provider gave, its codes, access tokens and ID tokens.
fn secrets(google: &Google, people: &[&Person]) -> Vec<String> {
    let mut secrets = vec![CLIENT_SECRET.to_owned()];
    secrets.extend(people.iter().map(|person| person.email.clone()));
    secrets.extend(google.provider.codes());
    for issued in google.provider.issued() {
        secrets.push(issued.access_token);
        secrets.push(issued.id_token);
    }
    secrets
}

#[test]
fn database_holds_no_email_google_token_or_client_secret() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986900", "ada.db@example.com");
    let bob = google.person("110248495921238986901", "bob.db@example.com");
    google.signed_up(&browser, &ada);
    google.signed_up(&browser, &bob);
    let moved = google.person(&ada.subject, "ada.moved.db@example.org");
    google.signed_up(&browser, &moved);
    assert_eq!(
        google.provider.issued().len(),
        3,
        "each sign-in got its tokens"
    );

    let rows = google.db.rows();
    assert!(!rows.is_empty(), "the database holds the accounts");
    assert_holds_none(&rows, &secrets(&google, &[&ada, &bob, &moved]));
}

/// `output` holds none of `secrets`.
#[track_caller]
fn shows_none(output: &str, secrets: &[String]) {
    for secret in secrets {
        assert!(!output.contains(secret.as_str()), "{secret:?} in {output}");
    }
}

#[test]
fn server_output_never_shows_the_secret_an_email_an_id_token_or_a_code() {
    let secret = [(SECRET_ENV, CLIENT_SECRET)];
    let (_, help) = run(&["--help"], &secret);
    assert!(
        help.contains(SECRET_ENV),
        "--help names the setting: {help}"
    );
    assert!(!help.contains(CLIENT_SECRET), "{help}");

    // A start that fails on a bad setting.
    let db = TestDb::create();
    let url = db.url();
    let mut args = vec!["--listen", "127.0.0.1:0", "--database-url", &url];
    args.extend(["--public-url", "not a URL", "--google-client-id", CLIENT_ID]);
    args.extend(["--google-issuer", "http://127.0.0.1:9"]);
    let (started, output) = run(&args, &secret);
    assert!(!started, "{output}");
    assert!(!output.contains(CLIENT_SECRET), "{output}");

    // A sign-in, and sign-ins that the provider fails.
    let browser = Browser::launch();
    let mut google = Google::start();
    let ada = google.person("110248495921238986910", "ada.output@example.com");
    let bob = google.person("110248495921238986911", "bob.output@example.com");
    google.signed_up(&browser, &ada);
    for fault in [
        Fault::InvalidGrant,
        Fault::TokenStatus { status: 500 },
        Fault::AccessDenied,
        Fault::Nonce {
            nonce: "n-another-sign-in-0123456789".to_owned(),
        },
    ] {
        google.provider.set_faults(&[fault]);
        let page = browser.page();
        google.sign_in(&page, &bob);
        google.assert_signed_out(&page);
    }
    // A sign-in while the provider is down.
    google.provider.set_faults(&[]);
    let held = browser.page();
    let callback = google.held_callback(&held, &bob);
    google.provider.stop();
    held.goto(&callback);
    google.assert_signed_out(&held);
    let shown = secrets(&google, &[&ada, &bob]);
    assert!(
        shown.len() > 4,
        "the provider gave codes and tokens: {shown:?}"
    );
    shows_none(&google.server.stop(), &shown);

    // A sign-in that the provider refuses for the secret.
    let mut refused = Google::launch(Setup {
        secret: Some(WRONG_SECRET),
        ..Setup::default()
    });
    let grace = refused.person("110248495921238986912", "grace.output@example.com");
    let page = browser.page();
    refused.sign_in(&page, &grace);
    refused.assert_signed_out(&page);
    let mut shown = secrets(&refused, &[&grace]);
    shown.push(WRONG_SECRET.to_owned());
    shows_none(&refused.server.stop(), &shown);
}
