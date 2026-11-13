//! The browser's way through a Google sign-in: it returns only to a page of this server, and under
//! an https public address its cookies go only over HTTPS.

use super::common::browser::{Browser, Cookie, Page};
use super::common::sign_in::{assert_only_over_https, cookies_set_by, off_server};
use super::{CALLBACK, Google, HOST, Setup};
use skald_oidc_stand_in::Person;

#[test]
fn return_address_stays_on_this_server() {
    let browser = Browser::launch();
    let google = Google::start();
    let ada = google.person("110248495921238986800", "ada.return@example.com");
    let elsewhere = browser.echo(false);
    let server = format!("{}/", google.public);
    let provider = format!("{}/", google.provider.issuer());

    for to in &off_server(HOST, elsewhere) {
        let page = browser.page();
        google.provider_session(&page, &ada);
        let visit = page.goto_settled(&google.start_url(Some(to)));
        assert!(
            visit
                .navigations
                .iter()
                .all(|url| url.starts_with(&server) || url.starts_with(&provider)),
            "return_to {to:?}: the browser left the server: {visit:?}"
        );
        assert_eq!(visit.last(), server, "return_to {to:?}: {visit:?}");
        google.account(&page);
    }
    assert!(
        browser.hits(elsewhere).is_empty(),
        "the browser went to another port"
    );

    // A page of this server is where the browser returns.
    let page = browser.page();
    google.provider_session(&page, &ada);
    let visit = page.goto_settled(&google.start_url(Some("/settings?tab=devices")));
    assert_eq!(
        visit.last(),
        format!("{}/settings?tab=devices", google.public),
        "{visit:?}"
    );
    google.account(&page);

    // Without a return address, it returns to this server too.
    let page = browser.page();
    google.provider_session(&page, &ada);
    let visit = page.goto_settled(&google.start_url(None));
    assert!(visit.last().starts_with(&server), "{visit:?}");
}

/// Signs `person` in on `page` and gives the cookies of the server's host that the callback set or
/// changed: the session's.
fn session_cookies(google: &Google, page: &Page, person: &Person) -> Vec<Cookie> {
    let callback = google.held_callback(page, person);
    let set = cookies_set_by(page, HOST, &callback);
    google.account(page);
    set
}

#[test]
fn https_public_address_sends_both_cookies_only_over_https() {
    let browser = Browser::launch();
    let google = Google::launch(Setup {
        https: Some(&browser),
        ..Setup::default()
    });
    let ada = google.person("110248495921238986801", "ada.https@example.com");

    // The cookie that binds `state`, set when the browser leaves for the provider: the person is
    // not signed in there, so the browser stays at the provider.
    let page = browser.page();
    let visit = page.goto(&google.start_url(None));
    let authorize = format!("{}?", google.provider.authorize_url());
    assert!(
        visit.url.starts_with(&authorize),
        "at the provider: {visit:?}"
    );
    let state_cookies = page.cookies_of(HOST);
    assert!(!state_cookies.is_empty(), "the sign-in start sets a cookie");

    // The session's cookie.
    let signed_in = browser.page();
    let session = session_cookies(&google, &signed_in, &ada);
    assert_only_over_https(
        &browser,
        (HOST, CALLBACK),
        (&page, &state_cookies),
        (&signed_in, &session),
    );
    google.account(&signed_in);
}
