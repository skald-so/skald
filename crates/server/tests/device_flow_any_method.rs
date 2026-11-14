//! Acceptance tests of the device flow with any sign-in method: a machine signs in by device flow
//! whichever provider its person signs in with on the verification page. `skald-server` runs
//! against the OIDC stand-in, which plays Google, and the GitHub stand-in, with the person's part in
//! headless Chrome. Every response to the machine is checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The server, as these tests use it, beside `common/google.rs` and `common/device_flow.rs`:
//! - the device flow, `POST /auth/device`, `POST /auth/token` and the verification page
//!   `GET /device` and `POST /device`, works when the server has the settings of any provider;
//! - the verification page without a session offers sign-in with each provider the server has
//!   settings for, as a link or button whose text names the provider, GitHub or Google; each
//!   returns to the verification page with its code;
//! - an approval counts for the account of the web session, whatever method signed it in.

mod common;

use common::accounts::Who;
use common::browser::{
    APPROVE, Browser, CODE_FIELD, Page, SIGN_IN_GITHUB, SIGN_IN_GOOGLE, SUBMIT, Visit,
};
use common::device_flow::{DeviceFlow, Grant, INTERVAL, Machine, assert_ok, text};
use common::google::{Google, Setup};
use common::sign_in::page_session;
use common::spec::spec;
use common::{get_url, post_form};
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;

/// The verification page's path.
const PAGE: &str = "/device";
/// The page of the account's sign-in methods, and its button that links GitHub.
const METHODS_PAGE: &str = "/sign-in-methods";
const LINK_GITHUB: &str = "role=button[name=/link.*github/i]";

/// A server with Google's settings, and GitHub's too if `github`, whose codes poll every second.
fn server(github: bool) -> Google {
    Google::launch(Setup {
        github,
        args: &[INTERVAL, "1"],
        ..Setup::default()
    })
}

/// Signs `page`'s browser in to the OIDC stand-in as `person`, as a person does before a sign-in.
fn at_google(google: &Google, page: &Page, person: &Person) {
    google.provider_session(page, person);
}

/// Signs `page`'s browser in to the GitHub stand-in as `user`.
fn at_github(google: &Google, page: &Page, user: &User) {
    let at = google.github().sign_in_url(user.id);
    assert_eq!(page.goto(&at).status, Some(200), "{at}");
}

/// The person at `page`, without a session on the server at `public` and signed in to the provider
/// whose sign-in `link` finds, opens `grant`'s verification page with its code and signs in there
/// with that provider; fails unless the page offers it and the browser comes back to the page with
/// the code filled in or shown.
fn sign_in_on_the_page(page: &Page, public: &str, grant: &Grant, link: &str) -> Visit {
    let opened = page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(opened.status, Some(200), "{opened:?}");
    assert_eq!(
        page.count(link),
        1,
        "the page offers {link}: {}",
        page.text()
    );
    let visit = page.click_settled(&format!("{link} >> nth=0"));
    assert!(
        visit.url.starts_with(&format!("{public}{PAGE}")),
        "back on the verification page: {visit:?}"
    );
    let shown = page.values(CODE_FIELD).contains(&grant.user_code)
        || page.text().contains(&grant.user_code);
    assert!(shown, "the page has the code: {}", page.text());
    visit
}

/// The person at `page`, on the verification page with `grant`'s code after signing in there,
/// approves it; gives the session token the machine then gets.
fn approve_here(machine: &Machine, page: &Page, grant: &Grant) -> String {
    machine.assert_pending(grant);
    if page.count(APPROVE) == 0 {
        assert_eq!(page.click(SUBMIT).status, Some(200), "{}", page.text());
    }
    assert_ok(&page.click(APPROVE));
    machine.token(grant)
}

/// The person at `page`, signed in to the OIDC stand-in, signs in with Google on `grant`'s
/// verification page and approves the code; fails unless the machine's token acts as the account of
/// the browser's session, a Google account without a GitHub login. Gives that account.
fn google_approves(google: &Google, machine: &Machine, page: &Page, grant: &Grant) -> Who {
    sign_in_on_the_page(page, &google.public, grant, SIGN_IN_GOOGLE);
    let token = approve_here(machine, page, grant);
    let who = machine.who_of(&token);
    assert_eq!(who, account(page, &google.public));
    assert_eq!(who.github_login, None, "a Google account: {who:?}");
    who
}

/// The account of `page`'s session on the server at `public`.
fn account(page: &Page, public: &str) -> Who {
    let (status, body) = page_session(page, public);
    assert_eq!(status, 200, "GET /session: {body}");
    Who::of_body(&body)
}

/// A person without a session signs in with Google on the verification page and approves: the
/// machine's token acts as their account, created by that sign-in, which has no GitHub login.
#[test]
fn machine_signs_in_while_its_person_signs_in_with_google_on_the_page() {
    let browser = Browser::launch();
    let google = server(true);
    let machine = Machine::at(google.server.base());
    let ada = google.person("110248495921238989000", "ada.device@example.com");
    let grant = machine.authorize();

    let page = browser.page();
    google.assert_signed_out(&page);
    at_google(&google, &page, &ada);
    let who = google_approves(&google, &machine, &page, &grant);

    assert_eq!(who.username, "ada-device");
}

/// On a server with Google's settings only, the device flow works: the machine gets its codes, the
/// page offers Google sign-in and no GitHub one, and the approval gives the machine a token of the
/// person's account.
#[test]
fn google_only_server_signs_a_machine_in() {
    let browser = Browser::launch();
    let google = server(false);
    let machine = Machine::at(google.server.base());
    let ada = google.person("110248495921238989010", "ada.only@example.com");
    let grant = machine.authorize();
    assert!(
        grant
            .verification_uri
            .starts_with(&format!("{}{PAGE}", google.public)),
        "{grant:?}"
    );
    machine.assert_pending(&grant);

    let page = browser.page();
    at_google(&google, &page, &ada);
    page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(page.count(SIGN_IN_GITHUB), 0, "{}", page.text());
    let who = google_approves(&google, &machine, &page, &grant);

    // A second machine of the same person, whose browser is signed in by now.
    let second = machine.authorize();
    assert_eq!(machine.who_of(&machine.approve(&page, &second)), who);
}

/// An account with a Google and a GitHub method: a code approved after a sign-in with either on the
/// page counts for that one account.
#[test]
fn approval_counts_for_the_sessions_account_whichever_method_signed_it_in() {
    let browser = Browser::launch();
    let google = server(true);
    let machine = Machine::at(google.server.base());
    let ada = google.person("110248495921238989020", "ada.both@example.com");
    let bob = google.github_user(5_310_449_021, "ada-both-standin", "ada.gh@example.org");

    // Ada signs up with Google and links GitHub on the page of her sign-in methods.
    let page = browser.page();
    google.sign_in(&page, &ada);
    let who = google.account(&page);
    at_github(&google, &page, &bob);
    let visit = page.goto(&format!("{}{METHODS_PAGE}", google.public));
    assert_eq!(visit.status, Some(200), "{visit:?}");
    page.click_settled(LINK_GITHUB);
    let other = browser.page();
    google.github_sign_in(&other, &bob);
    assert_eq!(
        google.account(&other).account,
        who.account,
        "GitHub is linked"
    );

    // A machine approved after a GitHub sign-in on the page, and one after a Google sign-in.
    let with_github = browser.page();
    at_github(&google, &with_github, &bob);
    let first = machine.authorize();
    sign_in_on_the_page(&with_github, &google.public, &first, SIGN_IN_GITHUB);
    let first_token = approve_here(&machine, &with_github, &first);

    let with_google = browser.page();
    at_google(&google, &with_google, &ada);
    let second = machine.authorize();
    sign_in_on_the_page(&with_google, &google.public, &second, SIGN_IN_GOOGLE);
    let second_token = approve_here(&machine, &with_google, &second);

    assert_eq!(machine.who_of(&first_token).account, who.account);
    assert_eq!(machine.who_of(&second_token).account, who.account);
    assert_ne!(first_token, second_token);
}

/// Without a session, the page offers sign-in with exactly the providers the server has settings
/// for, each returning to the page with its code.
#[test]
fn page_offers_only_the_providers_the_server_has_settings_for() {
    let browser = Browser::launch();

    // GitHub's settings only.
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(5_310_449_031, "ada-offers-standin");
    let grant = flow.authorize();
    let page = browser.page();
    flow.stand.github_session(&page, &ada);
    page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(page.count(SIGN_IN_GOOGLE), 0, "{}", page.text());
    sign_in_on_the_page(&page, &flow.stand.public, &grant, SIGN_IN_GITHUB);

    // Google's only.
    let google = server(false);
    let machine = Machine::at(google.server.base());
    let bob = google.person("110248495921238989032", "bob.offers@example.com");
    let grant = machine.authorize();
    let page = browser.page();
    at_google(&google, &page, &bob);
    page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(page.count(SIGN_IN_GITHUB), 0, "{}", page.text());
    sign_in_on_the_page(&page, &google.public, &grant, SIGN_IN_GOOGLE);

    // Both: each provider's sign-in, from a browser without a session.
    let both = server(true);
    let machine = Machine::at(both.server.base());
    let carol = both.person("110248495921238989033", "carol.offers@example.com");
    let dan = both.github_user(5_310_449_034, "dan-offers-standin", "dan@example.org");
    let grant = machine.authorize();
    let page = browser.page();
    at_google(&both, &page, &carol);
    sign_in_on_the_page(&page, &both.public, &grant, SIGN_IN_GOOGLE);
    assert_eq!(account(&page, &both.public).github_login, None);
    let page = browser.page();
    at_github(&both, &page, &dan);
    page.goto_settled(&grant.verification_uri_complete);
    assert_eq!(page.count(SIGN_IN_GOOGLE), 1, "{}", page.text());
    sign_in_on_the_page(&page, &both.public, &grant, SIGN_IN_GITHUB);
    let who = account(&page, &both.public);
    assert_eq!(who.github_login.as_deref(), Some(dan.login.as_str()));
}

/// On a server with Google's settings only, the device flow's responses conform to the spec: the
/// machine's requests, the verification page with and without a session, and an approval.
#[test]
fn google_only_device_flow_responses_conform_to_spec() {
    let google = Google::launch(Setup {
        local: true,
        args: &[INTERVAL, "1"],
        ..Setup::default()
    });
    let machine = Machine::at(google.server.base());
    let ada = google.person("110248495921238989040", "ada.devspec@example.com");
    let (jar, callback) = google.to_callback(&ada);
    let jar = google.finish(&jar, &callback);
    let cookie = jar.header();

    let grant = machine.authorize();
    machine.assert_pending(&grant);
    for url in [&grant.verification_uri, &grant.verification_uri_complete] {
        for headers in [vec![], vec![("cookie", cookie.as_str())]] {
            let response = get_url(url, &headers);
            assert_eq!(response.status, 200, "GET {url}: {}", text(&response));
            machine.conforms("get", PAGE, &response);
        }
    }

    let approved = post_form(
        &format!("{}{PAGE}", google.server.base()),
        &[("cookie", &cookie), ("origin", &google.public)],
        &[("user_code", &grant.user_code), ("decision", "approve")],
    );
    assert_eq!(approved.status, 200, "{}", text(&approved));
    machine.conforms("post", PAGE, &approved);
    let token = machine.token(&grant);
    assert_eq!(machine.who_of(&token), Who::of(&google.session_of(&jar)));
}

/// The spec's verification page offers sign-in with Google as well as GitHub.
#[test]
fn spec_says_the_verification_page_offers_every_provider() {
    let spec = spec();
    let page = &spec["paths"][PAGE]["get"];
    let text = page.to_string();
    for provider in ["GitHub", "Google"] {
        assert!(
            text.contains(provider),
            "GET {PAGE} names {provider}: {text}"
        );
    }
}
