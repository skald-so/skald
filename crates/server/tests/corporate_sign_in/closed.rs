use skald_github_stand_in::User;
use skald_oidc_stand_in::{Key, Person, Provider};

use super::common::browser::{Browser, Page, Visit};
use super::common::google;
use super::common::sign_in::{self, page_session};
use super::flow::account;
use super::{PROVIDERS, START, Site, TestDb, api};

#[test]
fn corporate_browser_sign_in_creates_an_account_with_closed_sign_up_and_empty_lists() {
    let db = TestDb::create();
    let corporate = Provider::start(&[Key::rsa()]);
    let site = Site::launch(&db, &corporate, None, false, None, "closed");
    let providers = api(&site.server, PROVIDERS, "");
    assert_eq!(providers["sign_up"], "closed", "{providers}");
    assert_eq!(providers["providers"], serde_json::json!(["corporate"]));

    let browser = Browser::launch();
    let page = browser.page();
    let employee = Person::new("new-employee", "new-employee@example.test");
    let visit = site.sign_in(&page, &corporate, &employee, START);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    let employee_account = account(&page, &site, "corporate");
    assert_eq!(employee_account.username, "new-employee");
}

fn github_sign_in(site: &Site, page: &Page, user: &User) -> Visit {
    let github = site.github.as_ref().expect("GitHub stand-in");
    github.put_user(user.clone());
    sign_in::github_session(page, github, user);
    page.goto_settled(&sign_in::start_url(
        &site.web,
        sign_in::START,
        Some("/health"),
    ))
}

#[track_caller]
fn assert_closed(site: &Site, page: &Page, visit: &Visit) {
    assert_eq!(visit.status, Some(403), "{visit:?}: {}", page.text());
    let html = page.evaluate("document.documentElement.outerHTML");
    assert!(
        html.as_str()
            .expect("the page's HTML")
            .contains("https://skald.so/problems/sign-up-closed"),
        "{}",
        page.text()
    );
    let (status, body) = page_session(page, &site.web);
    assert_eq!(status, 401, "GET /session: {body}");
}

#[test]
fn corporate_enabled_does_not_change_closed_sign_up_for_github_and_google() {
    let db = TestDb::create();
    let corporate = Provider::start(&[Key::rsa()]);
    let google = Provider::start(&[Key::rsa()]);
    let browser = Browser::launch();
    let github_user = User::new(73_015_541, "existing-octocat");
    let google_user = Person::new("existing-google-user", "existing-google@example.test");
    let accounts = |site: &Site| {
        let github_page = browser.page();
        github_sign_in(site, &github_page, &github_user);
        let google_page = browser.page();
        site.sign_in(&google_page, &google, &google_user, google::START);
        [
            account(&github_page, site, "github"),
            account(&google_page, site, "google"),
        ]
    };

    let open = Site::start(&db, &corporate, Some(&google), true);
    let before = accounts(&open);
    assert_ne!(before[0].account, before[1].account);
    drop(open);

    let closed = Site::launch(&db, &corporate, Some(&google), true, None, "closed");
    assert_eq!(accounts(&closed), before);

    let github_page = browser.page();
    let github_user = User::new(73_015_540, "unlisted-octocat");
    let visit = github_sign_in(&closed, &github_page, &github_user);
    assert_closed(&closed, &github_page, &visit);

    let google_page = browser.page();
    let google_user = Person::new("unlisted-google-user", "unlisted-google@example.test");
    let visit = closed.sign_in(&google_page, &google, &google_user, google::START);
    assert_closed(&closed, &google_page, &visit);
}
