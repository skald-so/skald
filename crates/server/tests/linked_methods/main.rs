//! Acceptance tests of linking a sign-in method: a signed-in person links another GitHub or Google
//! method to their account, and either method then signs in to it. `skald-server` runs against the
//! OIDC stand-in, which plays Google, and the GitHub stand-in with its callback set, with the
//! person's part in headless Chrome, so cookies act as in a browser. Responses are checked against
//! the OpenAPI spec in `api/openapi.json`.
//!
//! The server, as these tests use it, beside `common/google.rs`:
//! - `GET /sign-in-methods`: the page of the session's account's sign-in methods. It names each
//!   method's provider, GitHub or Google, when it was linked and, for GitHub, its login; it has a
//!   button "Link GitHub" and one "Link Google", each only while the server has that provider's
//!   settings, each submitting a form;
//! - `POST /auth/github/link` and `POST /auth/google/link`: the forms of those buttons, with no
//!   field the page has to fill in. With a web session from a sign-in more recent than the setting
//!   `--link-sign-in-age <SECONDS>`, a request from a page of this server, as its `Origin` says,
//!   sends the browser to the provider's authorization endpoint as a sign-in does, `Location` and
//!   `state` cookie alike; Google's with `prompt=select_account`. The provider comes back through
//!   the sign-in callback, `/auth/github/callback` or `/auth/google/callback`, which adds the
//!   method to that session's account instead of signing in. The callback may answer a page that
//!   refreshes itself to a page of this server with `<meta http-equiv="refresh">`, as a sign-in's
//!   does, and finish the link there; the tests follow such refreshes. With an older session, the
//!   answer is a page of this server that asks to sign in again, with a link or button named
//!   "sign in";
//! - `GET /account/sign-in-methods`: the session's account's sign-in methods, to a web or a device
//!   session, stating the protocol version: `{"methods": [{"provider": "github" | "google",
//!   "linked_at": <RFC 3339 date-time>, "github_login": <login, GitHub only>}]}`;
//! - GitHub sign-ins and links send an S256 PKCE challenge, and exchange the code with its
//!   verifier.

#[path = "../common/mod.rs"]
mod common;

mod links;
mod lists;
mod pkce;
mod refusals;
mod spec;

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;

use common::accounts::Who;
use common::browser::{Browser, Page, Visit};
use common::google::{Google, Setup};
use common::sign_in::{github_authorize_link, github_rows, page_get};

/// The page of the account's sign-in methods.
pub const METHODS_PAGE: &str = "/sign-in-methods";
/// The account's sign-in methods in the API.
pub const METHODS: &str = "/account/sign-in-methods";
/// Where the page's forms start a link.
pub const GITHUB_LINK: &str = "/auth/github/link";
pub const GOOGLE_LINK: &str = "/auth/google/link";
/// The setting: how recent a web session's sign-in has to be to start a link, in seconds.
pub const LINK_AGE: &str = "--link-sign-in-age";
/// The page's buttons that start a link.
pub const LINK_GITHUB: &str = "role=button[name=/link.*github/i]";
pub const LINK_GOOGLE: &str = "role=button[name=/link.*google/i]";

/// A sign-in method as the API lists it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    /// A GitHub method and its login.
    GitHub(String),
    Google,
}

/// A server with both providers' settings and the extra arguments `args`; its public address is
/// plain HTTP under `skald.test`.
pub fn both(args: &[&str]) -> Google {
    Google::launch(Setup {
        github: true,
        args,
        ..Setup::default()
    })
}

/// Seconds since the Unix epoch.
fn now() -> i64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.expect("a clock after 1970").as_secs() as i64
}

/// Seconds since the Unix epoch of `text`, an RFC 3339 date-time such as
/// `2026-11-14T09:30:00.123456Z` or `2026-11-14T10:30:00+01:00`.
pub fn rfc3339(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let number = |from: usize, len: usize| -> Option<i64> {
        let digits = text.get(from..from + len)?;
        digits
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| digits.parse().ok())?
    };
    let separators = [(4, b'-'), (7, b'-'), (13, b':'), (16, b':')];
    if bytes.len() < 20
        || separators.iter().any(|(at, byte)| bytes[*at] != *byte)
        || !matches!(bytes[10], b'T' | b't')
    {
        return None;
    }
    let (year, month, day) = (number(0, 4)?, number(5, 2)?, number(8, 2)?);
    let (hour, minute, second) = (number(11, 2)?, number(14, 2)?, number(17, 2)?);
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            let (hours, minutes) = (number(text.len() - 5, 2)?, number(text.len() - 2, 2)?);
            sign * (hours * 3600 + minutes * 60)
        }
    };
    // Days since 1970-01-01 of the proleptic Gregorian date: Howard Hinnant's days_from_civil.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + of_era * 365 + of_era / 4 - of_era / 100 + of_year - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// The methods in `body`, a `200` answer to `GET /account/sign-in-methods`, in order; each linked
/// within the last hour.
pub fn methods_in(body: &Value) -> Vec<Method> {
    let listed = body["methods"]
        .as_array()
        .unwrap_or_else(|| panic!("no methods in {body}"));
    let now = now();
    let mut methods: Vec<Method> = listed
        .iter()
        .map(|item| {
            let linked = linked_at(item);
            assert!(
                (now - 3600..=now + 60).contains(&linked),
                "linked within the last hour, now {now}: {item}"
            );
            match (item["provider"].as_str(), item.get("github_login")) {
                (Some("github"), Some(Value::String(login))) => Method::GitHub(login.clone()),
                (Some("google"), None) => Method::Google,
                _ => panic!("a GitHub method with its login or a Google one without: {item}"),
            }
        })
        .collect();
    methods.sort();
    methods
}

/// When `item`, a method in a `GET /account/sign-in-methods` answer, was linked.
pub fn linked_at(item: &Value) -> i64 {
    let text = item["linked_at"].as_str();
    text.and_then(rfc3339)
        .unwrap_or_else(|| panic!("linked_at is an RFC 3339 date-time: {item}"))
}

/// `GET /account/sign-in-methods` from a page of the server in `page`'s browser; fails without a
/// session. Gives the body.
pub fn listed(google: &Google, page: &Page) -> Value {
    let (status, body) = page_get(page, &google.public, METHODS);
    assert_eq!(status, 200, "GET {METHODS}: {body}");
    body
}

/// The sign-in methods of the account of `page`'s session.
pub fn methods(google: &Google, page: &Page) -> Vec<Method> {
    methods_in(&listed(google, page))
}

/// The methods of a GitHub method of `user` alone.
pub fn github_only(user: &User) -> Vec<Method> {
    vec![Method::GitHub(user.login.clone())]
}

/// The methods of a GitHub method of `user` and a Google one, in order.
pub fn github_and_google(user: &User) -> Vec<Method> {
    vec![Method::GitHub(user.login.clone()), Method::Google]
}

/// Opens the page of the account's sign-in methods in `page`'s browser.
pub fn open_methods_page(google: &Google, page: &Page) -> Visit {
    let visit = page.goto(&format!("{}{METHODS_PAGE}", google.public));
    assert_eq!(visit.status, Some(200), "GET {METHODS_PAGE}: {visit:?}");
    visit
}

/// Clicks `button`, [`LINK_GITHUB`] or [`LINK_GOOGLE`], on the page of the account's sign-in
/// methods in `page`'s browser; gives the visit it starts.
pub fn click_link(google: &Google, page: &Page, button: &str) -> Visit {
    open_methods_page(google, page);
    assert_eq!(page.count(button), 1, "the page offers {button}");
    page.click_settled(button)
}

/// Signs `page`'s browser in to GitHub as `user`, as a person does before a sign-in.
pub fn github_session(google: &Google, page: &Page, user: &User) {
    let visit = page.goto(&google.github().sign_in_url(user.id));
    assert_eq!(
        visit.status,
        Some(200),
        "GitHub stand-in sign-in: {visit:?}"
    );
}

/// Signs `page`'s browser in to GitHub as `user`, then links GitHub from the page of the account's
/// sign-in methods; gives the visit the link starts.
pub fn link_github(google: &Google, page: &Page, user: &User) -> Visit {
    github_session(google, page, user);
    click_link(google, page, LINK_GITHUB)
}

/// Signs `page`'s browser in to the provider as `person`, then links Google from the page of the
/// account's sign-in methods; gives the visit the link starts.
pub fn link_google(google: &Google, page: &Page, person: &Person) -> Visit {
    google.provider_session(page, person);
    click_link(google, page, LINK_GOOGLE)
}

/// Runs a link of the GitHub user `user` in `page`'s browser up to GitHub's page that asks to
/// authorize the app; gives the callback URL its link leads to, with `code` and `state`, not yet
/// opened.
pub fn held_github_link(google: &Google, page: &Page, user: &User) -> String {
    let visit = page.goto(&google.github().asking_sign_in_url(user.id));
    assert_eq!(
        visit.status,
        Some(200),
        "GitHub stand-in sign-in: {visit:?}"
    );
    let visit = click_link(google, page, LINK_GITHUB);
    github_authorize_link(google.github(), &google.public, page, &visit)
}

/// Signs `person` in with Google on a new page of `browser`; gives the page and the account.
pub fn google_page<'a>(google: &Google, browser: &'a Browser, person: &Person) -> (Page<'a>, Who) {
    let page = browser.page();
    google.sign_in(&page, person);
    let who = google.account(&page);
    (page, who)
}

/// Signs `user` in with GitHub on a new page of `browser`; gives the page and the account.
pub fn github_page<'a>(google: &Google, browser: &'a Browser, user: &User) -> (Page<'a>, Who) {
    let page = browser.page();
    google.github_sign_in(&page, user);
    let who = google.account(&page);
    (page, who)
}

/// Checks that the GitHub user `user` is in no row of the database: no account has them.
#[track_caller]
pub fn assert_no_github_account(google: &Google, user: &User, what: &str) {
    assert_eq!(
        github_rows(&google.db, user),
        Vec::<String>::new(),
        "{what}"
    );
}

/// Checks that `page`'s session is still that of `who`, an account with a Google method alone, and
/// that the GitHub user `user` has no account: a link of `user` linked nothing.
#[track_caller]
pub fn assert_unlinked(google: &Google, page: &Page, who: &Who, user: &User, what: &str) {
    assert_eq!(google.account(page).account, who.account, "{what}");
    assert_eq!(methods(google, page), vec![Method::Google], "{what}");
    assert_no_github_account(google, user, what);
}

/// Checks that `visit` ended on a page of the server at `google`'s public address.
#[track_caller]
pub fn assert_on_server(google: &Google, visit: &Visit) {
    assert!(
        visit.url.starts_with(&format!("{}/", google.public)),
        "back on the server: {visit:?}"
    );
}
