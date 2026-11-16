//! The method a web session signed in with: the server records it at the sign-in, and the session's
//! account names it, whatever other methods the account has.

use serde_json::Value;

use crate::common::Response;
use crate::common::device_flow::{json, text};
use crate::common::google::{Google, Setup};
use crate::common::sign_in::Jar;
use crate::common::spec::{operation_violations, spec};

/// `GET /session` of the web session in `jar`, which must be one; checked against the spec.
fn session(google: &Google, jar: &Jar) -> Value {
    let response: Response = google.session_of(jar);
    let violations = operation_violations(&spec(), "get", "/session", &response);
    assert_eq!(violations, Vec::<String>::new(), "{}", text(&response));
    assert_eq!(response.status, 200, "{}", text(&response));
    json(&response)
}

/// A Google sign-up's session names Google; after the account gets a GitHub method too, a GitHub
/// sign-in's session names GitHub and the Google session still names Google, each with the
/// account's GitHub login.
#[test]
fn web_session_names_the_method_it_signed_in_with() {
    let google = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });
    let person = google.person("220300001", "hal-methods@example.test");
    let (started, callback) = google.to_callback(&person);
    let by_google = google.finish(&started, &callback);
    let first = session(&google, &by_google);
    assert_eq!(first["sign_in_method"], "google", "{first}");
    assert_eq!(first.get("github_login"), None, "{first}");
    let account = first["account"].as_str().expect("an account").to_owned();

    // The account's GitHub method, as a link adds it.
    let user = google.github_user(3_220_300_001, "Hal-Methods-Standin", "hal@example.test");
    google.db.execute(&format!(
        "INSERT INTO sign_in_methods (provider, subject, account_id, login)
         VALUES ('github', '{}', '{account}', '{}')",
        user.id, user.login
    ));
    let (started, callback) = google.github_to_callback(&user);
    let by_github = google.finish(&started, &callback);
    let second = session(&google, &by_github);
    assert_eq!(second["account"], account.as_str(), "{second}");
    assert_eq!(second["sign_in_method"], "github", "{second}");
    assert_eq!(second["github_login"], "Hal-Methods-Standin", "{second}");

    let again = session(&google, &by_google);
    assert_eq!(again["account"], account.as_str(), "{again}");
    assert_eq!(again["sign_in_method"], "google", "{again}");
    assert_eq!(again["github_login"], "Hal-Methods-Standin", "{again}");
}

/// A GitHub sign-up's session names GitHub.
#[test]
fn github_sign_up_session_names_github() {
    let google = Google::launch(Setup {
        github: true,
        ..Setup::default()
    });
    let user = google.github_user(3_220_300_011, "Ivy-Methods-Standin", "ivy@example.test");
    let (started, callback) = google.github_to_callback(&user);
    let jar = google.finish(&started, &callback);
    let body = session(&google, &jar);
    assert_eq!(body["sign_in_method"], "github", "{body}");
    assert_eq!(body["github_login"], "Ivy-Methods-Standin", "{body}");
}
