//! While sign-up is closed, a sign-up creates an account only for a listed GitHub login, a listed
//! Google email that is verified, or a Google account whose `hd` is a listed domain.

use super::common::browser::Browser;
use super::common::get_url;
use super::common::spec::{operation_violations, spec};
use super::*;

/// The lists most tests close sign-up with.
const LISTS: Lists = Lists {
    logins: "ada-listed,someone-else",
    emails: "grace.listed@example.com,other@example.net",
    domains: "acme.example",
};

/// A listed GitHub login signs up: the session's account has that login.
#[test]
fn listed_github_login_signs_up() {
    let browser = Browser::launch();
    let google = closed(LISTS);
    let ada = github_user(&google, 7_310_000_001, "ada-listed");
    github_signed_up(&google, &browser.page(), &ada);
}

/// A listed Google email, verified, signs up.
#[test]
fn listed_google_email_signs_up() {
    let browser = Browser::launch();
    let google = closed(LISTS);
    let grace = person(
        &google,
        "110248495921238991001",
        "grace.listed@example.com",
        true,
        None,
    );
    let who = google.signed_up(&browser, &grace);
    assert_eq!(who.username, "grace-listed");
}

/// A Google account whose `hd` is a listed domain signs up, whatever its email.
#[test]
fn google_account_of_a_listed_domain_signs_up() {
    let browser = Browser::launch();
    let google = closed(LISTS);
    let cy = person(
        &google,
        "110248495921238991002",
        "cy@acme.example",
        true,
        Some("acme.example"),
    );
    google.signed_up(&browser, &cy);
    let dee = person(
        &google,
        "110248495921238991003",
        "dee@mail.acme-group.example",
        true,
        Some("acme.example"),
    );
    google.signed_up(&browser, &dee);
}

/// An unlisted GitHub login is refused, creating no account; so is a GitHub user whose public email
/// is on the email list but whose login is not: GitHub counts by login only.
#[test]
fn unlisted_github_logins_are_refused() {
    let browser = Browser::launch();
    let google = closed(LISTS);
    let bob = github_user(&google, 7_310_000_002, "bob-unlisted");
    let page = browser.page();
    let visit = github_visit(&google, &page, &bob);
    assert_refused(&google, &page, &visit, "an unlisted login");
    assert_no_github_account(&google, &bob, "an unlisted login");

    let eve = google.github_user(7_310_000_003, "eve-unlisted", "grace.listed@example.com");
    let page = browser.page();
    let visit = github_visit(&google, &page, &eve);
    assert_refused(
        &google,
        &page,
        &visit,
        "a listed email on an unlisted login",
    );
    assert_no_github_account(&google, &eve, "a listed email on an unlisted login");
}

/// Google accounts that are not listed are refused, creating no account: an unlisted email, an
/// unlisted `hd`, an email in a listed domain without `hd`, and a listed email that is not
/// verified.
#[test]
fn unlisted_google_accounts_are_refused() {
    let browser = Browser::launch();
    let google = closed(LISTS);
    let cases = [
        (
            "an unlisted email",
            person(
                &google,
                "110248495921238991010",
                "dan@example.org",
                true,
                None,
            ),
        ),
        (
            "an unlisted hd",
            person(
                &google,
                "110248495921238991011",
                "fay@other.example",
                true,
                Some("other.example"),
            ),
        ),
        (
            "an email in a listed domain without hd",
            person(
                &google,
                "110248495921238991012",
                "gil@acme.example",
                true,
                None,
            ),
        ),
        (
            "a listed email with email_verified false",
            person(
                &google,
                "110248495921238991013",
                "other@example.net",
                false,
                None,
            ),
        ),
    ];
    for (what, someone) in &cases {
        let page = browser.page();
        let visit = google.sign_in(&page, someone);
        assert_refused(&google, &page, &visit, what);
        google.assert_no_account(someone, what);
    }
}

/// Logins, emails and domains compare without case, both ways.
#[test]
fn lists_compare_without_case() {
    let browser = Browser::launch();
    let google = closed(Lists {
        logins: "Ada-Case,hal-case",
        emails: "Grace.Case@Example.COM,ivy.case@example.com",
        domains: "Acme-Case.Example,jot-case.example",
    });
    let ada = github_user(&google, 7_310_000_011, "ada-CASE");
    github_signed_up(&google, &browser.page(), &ada);
    let hal = github_user(&google, 7_310_000_012, "Hal-Case");
    github_signed_up(&google, &browser.page(), &hal);

    for (subject, email, domain) in [
        ("110248495921238991021", "grace.case@example.com", None),
        ("110248495921238991022", "Ivy.Case@EXAMPLE.com", None),
        (
            "110248495921238991023",
            "kim@acme-case.example",
            Some("ACME-case.example"),
        ),
        (
            "110248495921238991024",
            "lou@Jot-Case.Example",
            Some("Jot-Case.Example"),
        ),
    ] {
        let someone = person(&google, subject, email, true, domain);
        google.signed_up(&browser, &someone);
    }
}

/// A refused sign-up's answers conform to the OpenAPI spec, with GitHub and with Google, and name
/// the problem type; neither sets a session cookie.
#[test]
fn refusals_conform_to_the_spec() {
    let google = closed(LISTS);
    let spec = spec();
    let bob = github_user(&google, 7_310_000_021, "bob-spec");
    let dan = person(
        &google,
        "110248495921238991031",
        "dan.spec@example.org",
        true,
        None,
    );

    let (jar, callback) = google.github_to_callback(&bob);
    let github = get_url(&callback, &[("cookie", &jar.header())]);
    let (jar, callback) = google.to_callback(&dan);
    let oidc = get_url(&callback, &[("cookie", &jar.header())]);
    for (path, response) in [
        ("/auth/github/callback", &github),
        ("/auth/google/callback", &oidc),
    ] {
        assert_eq!(response.status, 403, "{path}: {}", shown(response));
        assert!(
            body_text(response).contains(REFUSED),
            "{path} names {REFUSED}: {}",
            shown(response)
        );
        assert_eq!(
            operation_violations(&spec, "get", path, response),
            Vec::<String>::new()
        );
        let session = response
            .header_values("set-cookie")
            .into_iter()
            .filter(|cookie| cookie.starts_with("skald_session="))
            .filter(|cookie| !cookie.starts_with("skald_session=;"))
            .count();
        assert_eq!(session, 0, "{path} sets a session: {:?}", response.headers);
    }
    assert_no_github_account(&google, &bob, "a refused GitHub sign-up");
    google.assert_no_account(&dan, "a refused Google sign-up");
}
