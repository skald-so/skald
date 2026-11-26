use std::collections::BTreeSet;

use skald_oidc_stand_in::{Endpoint, Fault, Key, Person, Provider};

use super::common::accounts::{Who, assert_valid};
use super::common::browser::{Browser, Page};
use super::common::sign_in::{self, Jar, page_session, provider_back};
use super::common::{TestDb, assert_holds_none};
use super::{
    CALLBACK, CLIENT_ID, CLIENT_SECRET, METHODS, SESSION, SESSIONS, START, Site, api,
    assert_conforms, assert_providers, text,
};

#[track_caller]
fn account(page: &Page, site: &Site, method: &str) -> Who {
    let (status, body) = page_session(page, &site.web);
    assert_eq!(status, 200, "GET {SESSION}: {body}");
    assert_eq!(body["sign_in_method"], method, "{body}");
    Who::of_body(&body)
}

#[test]
fn corporate_only_sign_in_returns_to_the_web_origin_and_keeps_the_account() {
    let db = TestDb::create();
    let provider = Provider::start_at("/realms/workforce", &[Key::rsa()]);
    let site = Site::start(&db, &provider, None, false);
    assert_providers(&site.server, &["corporate"]);
    assert_eq!(provider.metadata_reads(), 0, "discovery is on demand");
    let browser = Browser::launch();
    let person = Person::new("employee-ada", "Ada.Lovelace@example.test");
    let first = browser.page();
    let visit = site.sign_in(&first, &provider, &person, START);
    assert_eq!(visit.last(), format!("{}/health", site.web), "{visit:?}");
    let who = account(&first, &site, "corporate");
    assert_eq!(who.username, "ada-lovelace");
    assert_eq!(who.github_login, None);

    for email in [person.email.as_str(), "grace.moved@example.test"] {
        let next = browser.page();
        let same_employee = Person::new(&person.subject, email);
        site.sign_in(&next, &provider, &same_employee, START);
        assert_eq!(account(&next, &site, "corporate"), who, "{email}");
    }

    let other = browser.page();
    site.sign_in(
        &other,
        &provider,
        &Person::new("employee-new", &person.email),
        START,
    );
    let newcomer = account(&other, &site, "corporate");
    assert_ne!(newcomer.account, who.account);
    assert_eq!(newcomer.username, "ada-lovelace-2");
}

#[test]
fn same_subject_and_email_at_two_corporate_issuers_reach_different_accounts() {
    let db = TestDb::create();
    let one = Provider::start(&[Key::rsa()]);
    let other = Provider::start(&[Key::rsa()]);
    let person = Person::new("employee-123", "employee@example.test");
    let browser = Browser::launch();
    let sign_in = |provider| {
        let site = Site::start(&db, provider, None, false);
        let page = browser.page();
        site.sign_in(&page, provider, &person, START);
        account(&page, &site, "corporate")
    };

    let first = sign_in(&one);
    let second = sign_in(&other);
    assert_ne!(first.account, second.account);
    assert_eq!(sign_in(&one), first);
}

#[test]
fn an_identity_without_an_email_gets_a_username_and_can_sign_in_again() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    provider.set_faults(&[Fault::OmitEmail]);
    let site = Site::start(&db, &provider, None, false);
    let browser = Browser::launch();
    let person = Person::new("employee-456", "not-in-the-token@example.test");

    let first = browser.page();
    site.sign_in(&first, &provider, &person, START);
    let who = account(&first, &site, "corporate");
    assert_valid(&who.username);
    assert_ne!(who.username, "not-in-the-token");

    let next = browser.page();
    site.sign_in(&next, &provider, &person, START);
    assert_eq!(account(&next, &site, "corporate"), who);
}

#[test]
fn discovery_outage_leaves_github_and_google_available_and_corporate_recovers() {
    let db = TestDb::create();
    let mut corporate = Provider::start(&[Key::rsa()]);
    let google = Provider::start(&[Key::rsa()]);
    corporate.stop();
    let site = Site::start(&db, &corporate, Some(&google), true);
    site.server.assert_healthy();
    assert_providers(&site.server, &["corporate", "github", "google"]);
    let failure = site.server.get(START);
    assert!((400..600).contains(&failure.status), "{}", text(&failure));
    assert_eq!(failure.header("location"), None);
    assert_conforms(START, &failure);

    let browser = Browser::launch();
    let person = Person::new("employee-789", "mixed@example.test");
    let google_page = browser.page();
    site.sign_in(&google_page, &google, &person, super::common::google::START);
    let google_account = account(&google_page, &site, "google");

    let github = site.github.as_ref().expect("GitHub stand-in");
    let user = skald_github_stand_in::User::new(73_015_532, "mixed-octocat");
    github.put_user(user.clone());
    let github_page = browser.page();
    sign_in::github_session(&github_page, github, &user);
    github_page.goto(&format!("{}{}", site.web, sign_in::START));
    assert_eq!(
        account(&github_page, &site, "github")
            .github_login
            .as_deref(),
        Some(user.login.as_str())
    );

    corporate.restart();
    let corporate_page = browser.page();
    site.sign_in(&corporate_page, &corporate, &person, START);
    assert_ne!(
        account(&corporate_page, &site, "corporate").account,
        google_account.account
    );
    assert_eq!(account(&google_page, &site, "google"), google_account);
}

#[test]
fn start_callback_session_and_methods_match_openapi() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    let site = Site::start(&db, &provider, None, false);
    let person = Person::new("employee-901", "contract@example.test");
    provider.put_person(person.clone());

    let start = site.server.get(START);
    assert_eq!(start.status, 302, "GET {START}: {}", text(&start));
    assert_conforms(START, &start);
    let authorize = start.header("location").expect("an authorization redirect");
    assert!(authorize.starts_with(&format!("{}?", provider.authorize_url())));

    let mut jar = Jar::default();
    jar.take(&start);
    let callback = provider_back(&provider.sign_in_url(&person.subject), authorize);
    let callback_path = callback
        .strip_prefix(&site.web)
        .expect("the server's web origin");
    assert!(
        callback_path.starts_with(&format!("{CALLBACK}?")),
        "{callback}"
    );
    let done = site
        .server
        .get_with(callback_path, &[("cookie", &jar.header())]);
    assert_conforms(CALLBACK, &done);
    jar.take(&done);

    let authorize = provider.requests_to(Endpoint::Authorize);
    let params = &authorize[0].params;
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["redirect_uri"], format!("{}{CALLBACK}", site.web));
    assert_eq!(params["response_type"], "code");
    assert_eq!(
        params["scope"].split_whitespace().collect::<BTreeSet<_>>(),
        BTreeSet::from(["openid", "email"])
    );
    assert!(!params["state"].is_empty());
    assert!(!params["nonce"].is_empty());
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(sign_in::is_s256_challenge(&params["code_challenge"]));
    assert_eq!(
        provider.issued().len(),
        1,
        "the stand-in accepted the code and verifier"
    );

    let cookie = jar.header();
    let session = api(&site.server, SESSION, &cookie);
    assert_eq!(session["sign_in_method"], "corporate", "{session}");
    let body = api(&site.server, METHODS, &cookie);
    let methods = body["methods"].as_array().expect("a method list");
    assert_eq!(methods.len(), 1, "{body}");
    assert_eq!(methods[0]["provider"], "corporate", "{body}");
    assert_eq!(methods[0]["current"], true, "{body}");
    let body = api(&site.server, SESSIONS, &cookie);
    let sessions = body["sessions"].as_array().expect("a session list");
    assert_eq!(sessions.len(), 1, "{body}");
    assert_eq!(sessions[0]["kind"], "web", "{body}");
    assert_eq!(sessions[0]["sign_in_method"], "corporate", "{body}");
    assert_eq!(sessions[0]["current"], true, "{body}");
}

#[test]
fn invalid_provider_responses_create_no_account_or_session() {
    let db = TestDb::create();
    let provider = Provider::start(&[Key::rsa()]);
    let mut site = Site::start(&db, &provider, None, false);
    let browser = Browser::launch();
    let rejected = Person::new("employee-987", "rejected@example.test");

    let metadata = provider.metadata();
    let mut invalid = metadata.clone();
    invalid["issuer"] = serde_json::json!("https://another-provider.example.test");
    provider.set_metadata(&invalid);
    let response = site.server.get(START);
    assert!((400..600).contains(&response.status), "{}", text(&response));
    assert_eq!(response.header("location"), None);
    assert!(provider.requests_to(Endpoint::Authorize).is_empty());
    provider.set_metadata(&metadata);

    for fault in [
        Fault::InvalidGrant,
        Fault::TokenStatus { status: 503 },
        Fault::Nonce {
            nonce: "fixture-nonce-not-for-this-sign-in".to_owned(),
        },
    ] {
        let page = browser.page();
        let exchanges = provider.requests_to(Endpoint::Token).len();
        provider.set_faults(std::slice::from_ref(&fault));
        site.sign_in(&page, &provider, &rejected, START);
        let (status, body) = page_session(&page, &site.web);
        assert_eq!(status, 401, "{fault:?}: {body}");
        assert_eq!(
            provider.requests_to(Endpoint::Token).len(),
            exchanges + 1,
            "{fault:?}"
        );
    }
    assert_eq!(
        provider.issued().len(),
        1,
        "the bad-nonce test received an ID token"
    );
    provider.set_faults(&[]);

    // The first valid reply for the same subject supplies a different username. An account
    // created by a refused reply would keep the username from that reply.
    let accepted = Person::new(&rejected.subject, "accepted@example.test");
    let page = browser.page();
    site.sign_in(&page, &provider, &accepted, START);
    assert_eq!(account(&page, &site, "corporate").username, "accepted");
    let cookie = page
        .cookies_of("skald.test")
        .iter()
        .map(|cookie| cookie.pair())
        .collect::<Vec<_>>()
        .join("; ");
    let listed = api(&site.server, SESSIONS, &cookie);
    assert_eq!(listed["sessions"].as_array().unwrap().len(), 1, "{listed}");

    let mut private = vec![CLIENT_SECRET.to_owned(), accepted.email, rejected.email];
    private.extend(provider.codes());
    for issued in provider.issued() {
        private.extend([issued.id_token, issued.access_token]);
    }
    assert_holds_none(&[site.server.stop()], &private);
}
