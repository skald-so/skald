//! Links that link nothing: a method of another account, a session older than the setting, a link
//! callback in another browser, under another session, after its session ended, or with a reused
//! or expired `state`, and a link started from another site.

use std::thread;
use std::time::Duration;

use serde_json::json;
use skald_github_stand_in::Endpoint;

use super::common::browser::{Browser, Page, SIGN_IN_LINK, Visit};
use super::common::sign_in::{
    as_link, follow_from_site, github_authorize_link, param, submit_from_site,
};
use super::{
    GITHUB_LINK, LINK_AGE, LINK_GITHUB, Method, assert_no_github_account, assert_on_server,
    assert_unlinked, both, github_and_google, github_only, github_page, github_session,
    google_page, held_github_link, link_github, link_google, methods, open_methods_page,
};

#[test]
fn method_of_another_account_is_refused_and_neither_account_changes() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988100", "ada.other@example.com");
    let bob = google.github_user(5_310_448_101, "bob-other-standin", "bob.other@example.org");

    let (ada_page, ada_account) = google_page(&google, &browser, &ada);
    let (bob_page, bob_account) = github_page(&google, &browser, &bob);

    // Ada links Bob's GitHub, and Bob links Ada's Google.
    link_github(&google, &ada_page, &bob);
    link_google(&google, &bob_page, &ada);

    assert_eq!(google.account(&ada_page), ada_account);
    assert_eq!(google.account(&bob_page), bob_account);
    assert_eq!(methods(&google, &ada_page), vec![Method::Google]);
    assert_eq!(methods(&google, &bob_page), github_only(&bob));
    let (_, by_github) = github_page(&google, &browser, &bob);
    assert_eq!(by_github, bob_account);
    let (_, by_google) = google_page(&google, &browser, &ada);
    assert_eq!(by_google, ada_account);

    // A GitHub user of no account links.
    let carol = google.github_user(5_310_448_102, "carol-other-standin", "carol.o@example.org");
    link_github(&google, &ada_page, &carol);
    assert_eq!(methods(&google, &ada_page), github_and_google(&carol));
}

#[test]
fn session_older_than_the_setting_asks_to_sign_in_again_and_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[LINK_AGE, "15"]);
    let ada = google.person("110248495921238988110", "ada.old@example.com");
    let bob = google.github_user(5_310_448_111, "bob-old-standin", "bob.old@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    github_session(&google, &page, &bob);
    thread::sleep(Duration::from_secs(17));

    // The page's button if it still offers one, else the form it would submit.
    page.goto(&format!("{}{}", google.public, super::METHODS_PAGE));
    let visit = if page.count(LINK_GITHUB) > 0 {
        page.click_settled(LINK_GITHUB)
    } else {
        page.goto(&format!("{}/health", google.public));
        post_from_page(&page, &format!("{}{GITHUB_LINK}", google.public))
    };
    assert_on_server(&google, &visit);
    assert!(
        page.count(SIGN_IN_LINK) > 0,
        "asks to sign in again: {visit:?}"
    );
    assert_unlinked(&google, &page, &who, &bob, "an old session");

    // After a new sign-in, the link works.
    google.sign_in(&page, &ada);
    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
}

/// Submits a form of no fields to `action` by `POST` from the page open in `page`, as a page of the
/// server's own would.
fn post_from_page(page: &Page, action: &str) -> Visit {
    page.navigate_by(&format!(
        r#"setTimeout(() => {{
            const form = document.createElement("form");
            form.method = "post";
            form.action = {action};
            document.body.append(form);
            form.submit();
        }}, 0)"#,
        action = json!(action),
    ))
}

#[test]
fn link_callback_in_another_browser_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988120", "ada.browser@example.com");
    let carol = google.person("110248495921238988121", "carol.browser@example.com");
    let bob = google.github_user(5_310_448_121, "bob-browser-standin", "bob.br@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    let callback = held_github_link(&google, &page, &bob);

    // A browser with no session, and one with a session of another account.
    let fresh = browser.page();
    fresh.goto_settled(&callback);
    google.assert_signed_out(&fresh);
    let (lured, carol_account) = google_page(&google, &browser, &carol);
    lured.goto_settled(&callback);
    assert_eq!(google.account(&lured), carol_account);
    assert_eq!(methods(&google, &lured), vec![Method::Google]);

    assert_eq!(methods(&google, &page), vec![Method::Google]);
    assert_eq!(google.account(&page), who);
    assert_no_github_account(&google, &bob, "another browser");
}

#[test]
fn link_callback_under_another_session_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988130", "ada.session@example.com");
    let erin = google.person("110248495921238988131", "erin.session@example.com");
    let bob = google.github_user(5_310_448_131, "bob-session-standin", "bob.ss@example.org");

    let (page, _) = google_page(&google, &browser, &ada);
    let callback = held_github_link(&google, &page, &bob);
    // The same browser signs in to another account before the provider comes back.
    google.sign_in(&page, &erin);
    let erin_account = google.account(&page);
    page.goto_settled(&callback);

    assert_eq!(google.account(&page), erin_account);
    assert_eq!(methods(&google, &page), vec![Method::Google]);
    let (ada_page, _) = google_page(&google, &browser, &ada);
    assert_eq!(methods(&google, &ada_page), vec![Method::Google]);
    assert_no_github_account(&google, &bob, "another session");
}

#[test]
fn link_callback_after_its_session_ended_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988140", "ada.ended@example.com");
    let bob = google.github_user(5_310_448_141, "bob-ended-standin", "bob.end@example.org");

    let (page, who) = google_page(&google, &browser, &ada);
    let callback = held_github_link(&google, &page, &bob);
    // The server has no web sign-out: the session ends as its row goes.
    google.db.execute(&format!(
        "DELETE FROM sessions WHERE account_id = '{}'",
        who.account
    ));
    google.assert_signed_out(&page);
    page.goto_settled(&callback);

    google.assert_signed_out(&page);
    let (again, _) = google_page(&google, &browser, &ada);
    assert_eq!(methods(&google, &again), vec![Method::Google]);
    assert_no_github_account(&google, &bob, "an ended session");
}

#[test]
fn reused_link_state_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988150", "ada.reused@example.com");
    let bob = google.github_user(5_310_448_151, "bob-reused-standin", "bob.reu@example.org");
    let dan = google.github_user(5_310_448_152, "dan-reused-standin", "dan.reu@example.org");

    let (page, _) = google_page(&google, &browser, &ada);
    let callback = held_github_link(&google, &page, &bob);
    let state = param(&callback, "state").expect("the callback has a state");
    // The browser as it was before the callback: its session and the link's cookie.
    let jar = page.cookies();
    page.goto_settled(&callback);
    assert_eq!(methods(&google, &page), github_and_google(&bob));

    // The same browser state and `state` again, with a new code GitHub gives for that `state`.
    let again = browser.page();
    again.add_cookies(&jar);
    again.goto(&google.github().asking_sign_in_url(dan.id));
    let asked = google.github().requests_to(Endpoint::Authorize);
    let first = asked
        .iter()
        .find(|request| request.params.get("state") == Some(&state))
        .expect("the link's authorization request");
    let query = serde_urlencoded::to_string(&first.params).expect("a query");
    let github = google.github();
    let visit = again.goto(&format!("{}/login/oauth/authorize?{query}", github.url()));
    let replay = github_authorize_link(github, &google.public, &again, &visit);
    assert_eq!(param(&replay, "state"), Some(state));
    assert_ne!(param(&replay, "code"), param(&callback, "code"));
    again.goto_settled(&replay);

    assert_eq!(methods(&google, &again), github_and_google(&bob));
    assert_no_github_account(&google, &dan, "a reused state");
}

#[test]
fn expired_link_state_links_nothing() {
    let browser = Browser::launch();
    let google = both(&["--sign-in-state-lifetime", "5"]);
    let ada = google.person("110248495921238988160", "ada.expired@example.com");
    let bob = google.github_user(5_310_448_161, "bob-expired-standin", "bob.exp@example.org");
    let carol = google.github_user(
        5_310_448_162,
        "carol-expired-standin",
        "carol.e@example.org",
    );

    // Within its lifetime a link's state works.
    let (quick, _) = google_page(&google, &browser, &ada);
    link_github(&google, &quick, &carol);
    assert_eq!(methods(&google, &quick), github_and_google(&carol));

    let (slow, _) = google_page(&google, &browser, &ada);
    let callback = held_github_link(&google, &slow, &bob);
    thread::sleep(Duration::from_secs(7));
    slow.goto_settled(&callback);
    assert_eq!(methods(&google, &slow), github_and_google(&carol));
    assert_no_github_account(&google, &bob, "an expired state");
}

#[test]
fn link_started_from_another_site_links_nothing() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988170", "ada.site@example.com");
    let bob = google.github_user(5_310_448_171, "bob-site-standin", "bob.site@example.org");
    let other = format!("http://other.test:{}/", browser.echo(false));

    let (page, who) = google_page(&google, &browser, &ada);
    github_session(&google, &page, &bob);
    open_methods_page(&google, &page);
    let link = page.submission(LINK_GITHUB);

    // The page's own form, sent from another site: as a form, and as a link.
    submit_from_site(&page, &other, &link);
    assert_eq!(methods(&google, &page), vec![Method::Google], "as a form");
    follow_from_site(&page, &other, &as_link(&link));
    assert_eq!(methods(&google, &page), vec![Method::Google], "as a link");
    assert_eq!(google.account(&page), who);
    assert_no_github_account(&google, &bob, "another site");

    // The person's own link still works.
    link_github(&google, &page, &bob);
    assert_eq!(methods(&google, &page), github_and_google(&bob));
}
