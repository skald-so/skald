//! The account's sign-in methods: the page names them and when each was linked, offers a link for
//! each provider the server has settings for and, without a session, signs in back to itself; the
//! API gives the list to the account's sessions only.

use serde_json::Value;

use super::common::browser::{Browser, Page};
use super::common::device_flow::DeviceFlow;
use super::common::google::Google;
use super::common::sign_in::page_get;
use super::{
    LINK_GITHUB, LINK_GOOGLE, METHODS, METHODS_PAGE, Method, both, github_and_google, github_only,
    github_page, github_session, google_page, link_github, listed, methods, methods_in,
    open_methods_page,
};

/// Checks that the page of the account's sign-in methods in `page`'s browser offers a link to
/// GitHub if `github` and to Google if `google`, and names each of `names`.
#[track_caller]
fn assert_page(page: &Page, public: &str, (github, google): (bool, bool), names: &[&str]) {
    let visit = page.goto(&format!("{public}{METHODS_PAGE}"));
    assert_eq!(visit.status, Some(200), "GET {METHODS_PAGE}: {visit:?}");
    assert_eq!(
        page.count(LINK_GITHUB),
        usize::from(github),
        "{LINK_GITHUB}"
    );
    assert_eq!(
        page.count(LINK_GOOGLE),
        usize::from(google),
        "{LINK_GOOGLE}"
    );
    let text = page.text();
    for name in names {
        assert!(text.contains(name), "the page names {name:?}: {text:?}");
    }
}

/// Checks that the page of the account's sign-in methods in `page`'s browser names the day each of
/// the account's methods was linked, as the API gives it.
#[track_caller]
fn assert_page_dates(google: &Google, page: &Page) {
    let body = listed(google, page);
    let days: Vec<String> = body["methods"]
        .as_array()
        .expect("methods")
        .iter()
        .map(|item| item["linked_at"].as_str().expect("linked_at")[..10].to_owned())
        .collect();
    open_methods_page(google, page);
    let text = page.text();
    for day in days {
        assert!(text.contains(&day), "the page names {day}: {text:?}");
    }
}

/// Signs in on the page of the account's sign-in methods, without a session, by its link `link`,
/// after `provider_session` signs the browser in to the provider; checks that the sign-in returns
/// to that page; gives the page.
fn sign_in_from_methods_page<'a>(
    google: &Google,
    browser: &'a Browser,
    provider_session: impl Fn(&Page),
    link: &str,
) -> Page<'a> {
    let page = browser.page();
    provider_session(&page);
    open_methods_page(google, &page);
    assert_eq!(page.count(link), 1, "the page offers {link}");
    let visit = page.click_settled(link);
    let back = format!("{}{METHODS_PAGE}", google.public);
    assert_eq!(visit.url, back, "back on the page: {visit:?}");
    page
}

/// The methods of `body`, a `GET /account/sign-in-methods` answer, each as its JSON, in order.
fn items(body: &Value) -> Vec<String> {
    let listed = body["methods"].as_array().expect("methods");
    let mut items: Vec<String> = listed.iter().map(Value::to_string).collect();
    items.sort();
    items
}

#[test]
fn page_names_the_methods_and_offers_a_link_per_provider_with_settings() {
    let browser = Browser::launch();

    // Both providers.
    let google = both(&[]);
    let ada = google.person("110248495921238988300", "ada.page@example.com");
    let bob = google.github_user(5_310_448_301, "bob-page-standin", "bob.page@example.org");
    let (page, _) = google_page(&google, &browser, &ada);
    assert_page(&page, &google.public, (true, true), &["Google"]);
    link_github(&google, &page, &bob);
    assert_page(
        &page,
        &google.public,
        (true, true),
        &["Google", "GitHub", &bob.login],
    );
    assert_page_dates(&google, &page);

    // Google's settings only.
    let only_google = Google::start();
    let carol = only_google.person("110248495921238988301", "carol.page@example.com");
    let (page, _) = google_page(&only_google, &browser, &carol);
    assert_page(&page, &only_google.public, (false, true), &["Google"]);
    assert_eq!(methods(&only_google, &page), vec![Method::Google]);

    // GitHub's settings only.
    let flow = DeviceFlow::start();
    let dan = flow.stand.user(5_310_448_302, "dan-page-standin");
    let page = browser.page();
    flow.signed_in(&page, &dan);
    assert_page(
        &page,
        &flow.stand.public,
        (true, false),
        &["GitHub", &dan.login],
    );
    let (status, body) = page_get(&page, &flow.stand.public, METHODS);
    assert_eq!(status, 200, "{body}");
    assert_eq!(methods_in(&body), github_only(&dan));
}

#[test]
fn page_without_a_session_signs_in_and_returns_to_it() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988320", "ada.back@example.com");
    let bob = google.github_user(5_310_448_330, "bob-back-standin", "bob.back@example.org");

    let page = sign_in_from_methods_page(
        &google,
        &browser,
        |page| google.provider_session(page, &ada),
        "role=link[name=/sign in with google/i]",
    );
    assert_eq!(methods(&google, &page), vec![Method::Google]);
    let page = sign_in_from_methods_page(
        &google,
        &browser,
        |page| github_session(&google, page, &bob),
        "role=link[name=/sign in with github/i]",
    );
    assert_eq!(methods(&google, &page), github_only(&bob));
}

#[test]
fn only_the_accounts_web_sessions_get_its_list() {
    let browser = Browser::launch();
    let google = both(&[]);
    let ada = google.person("110248495921238988310", "ada.list@example.com");
    let bob = google.github_user(5_310_448_311, "bob-list-standin", "bob.list@example.org");
    let carol = google.github_user(5_310_448_312, "carol-list-standin", "carol.l@example.org");

    let (ada_page, _) = google_page(&google, &browser, &ada);
    link_github(&google, &ada_page, &bob);
    let (carol_page, _) = github_page(&google, &browser, &carol);

    assert_eq!(methods(&google, &ada_page), github_and_google(&bob));
    assert_eq!(methods(&google, &carol_page), github_only(&carol));
    // Another browser of Ada's reaches the same list.
    let (again, _) = google_page(&google, &browser, &ada);
    assert_eq!(
        items(&listed(&google, &again)),
        items(&listed(&google, &ada_page))
    );

    // No session: neither a browser without one nor a request without a cookie.
    let (status, body) = page_get(&browser.page(), &google.public, METHODS);
    assert_eq!(status, 401, "{body}");
    let version = skald_proto::PROTOCOL_VERSION.to_string();
    let bare = google
        .server
        .get_with(METHODS, &[(skald_proto::VERSION_HEADER, &version)]);
    assert_eq!(bare.status, 401);
}

#[test]
fn device_session_gets_its_accounts_list() {
    let browser = Browser::launch();
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(5_310_448_321, "ada-device-list-standin");
    let bob = flow.stand.user(5_310_448_322, "bob-device-list-standin");
    let page = browser.page();
    flow.signed_in(&page, &ada);
    let token = flow.enter_and_approve(&page, &flow.authorize());
    let bob_page = browser.page();
    flow.signed_in(&bob_page, &bob);

    let bearer = format!("Bearer {token}");
    let by_device = flow.request("get", METHODS, &[("authorization", &bearer)], None, true);
    assert_eq!(by_device.status, 200, "{:?}", by_device.body);
    let body: Value = serde_json::from_slice(&by_device.body).expect("JSON");
    assert_eq!(methods_in(&body), github_only(&ada));
    let (status, by_web) = page_get(&page, &flow.stand.public, METHODS);
    assert_eq!(status, 200);
    assert_eq!(
        body, by_web,
        "the device and the web session get the same list"
    );

    let wrong = format!("{bearer}x");
    let refused = flow.request("get", METHODS, &[("authorization", &wrong)], None, true);
    assert_eq!(refused.status, 401);
    let none = flow.request("get", METHODS, &[], None, true);
    assert_eq!(none.status, 401);
    // The session's account still answers as before.
    assert_eq!(flow.account_of(&token).github_login, ada.login);
}
