//! Sign-in methods on the web: a web session removes a sign-in method of its account, never the
//! last one, only from a page of this server and only after a recent sign-in; a removed method's
//! web sessions end, and the method may sign up anew or be linked to another account. A link
//! returns to the page that started it. Against a `skald-server` on PostgreSQL with the GitHub
//! stand-in and the OIDC stand-in, which plays Google, the person in headless Chrome; the web app
//! on the harness of `common/spa.rs`. Responses are checked against the OpenAPI spec in
//! `api/openapi.json`.
//!
//! The API, as these tests use it, beside `linked_methods/main.rs` and `web_sessions/main.rs`, each
//! request stating the protocol version:
//! - `GET /account/sign-in-methods`: each method also has `"method": <ID>`, which names it in the
//!   request that removes it and reveals nothing of the subject the provider gives the person;
//! - `DELETE /account/sign-in-methods/{method}`, with a web session's cookie, from a page of this
//!   server, as its `Origin` says, and a sign-in more recent than `--link-sign-in-age`: removes that
//!   method of the session's account, `204`. Anything else is a `4xx` with a problem details object
//!   and removes nothing: another site's `Origin` or none, a device session, an older sign-in, the
//!   account's last method, a method of another account or an unknown ID;
//! - `POST /auth/github/link` and `POST /auth/google/link` take `return_to` in the query, as
//!   `GET /auth/github` does: the link ends on that page of the web URL;
//! - `sessions.created_at` in the database is a session's sign-in.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../web_sessions/access.rs"]
mod access;
#[path = "../common/mod.rs"]
mod common;
#[path = "../web_sign_in/kit.rs"]
mod kit;
#[path = "../web_sessions/sessions.rs"]
mod sessions;

// The page helpers in `access.rs` and `account.rs` find these as siblings.
use common::{browser, spa};

mod account;

mod current;
mod ending;
mod pages;
mod refusals;
mod removal;
mod returns;
mod reuse;

use serde_json::Value;
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;

use common::Response;
use common::accounts::Who;
use common::browser::{Browser, Page, Visit};
use common::device_flow::{INTERVAL, Machine, json, text};
use common::google::{Google, Setup};
use sessions::{Api, cookie_of};

/// The account's sign-in methods: `GET`.
pub const METHODS: &str = "/account/sign-in-methods";
/// One sign-in method of the account: `DELETE`.
pub const ONE: &str = "/account/sign-in-methods/{method}";
/// The server's page of the account's sign-in methods, and its button that starts a GitHub link.
pub const METHODS_PAGE: &str = "/sign-in-methods";
pub const LINK_GITHUB: &str = "role=button[name=/link.*github/i]";
pub const LINK_GOOGLE: &str = "role=button[name=/link.*google/i]";
/// Where a GitHub link starts.
pub const GITHUB_LINK: &str = "/auth/github/link";
/// The setting: how recent a web session's sign-in has to be, in seconds.
pub const LINK_AGE: &str = "--link-sign-in-age";

/// A provider of sign-in methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Provider {
    GitHub,
    Google,
}

/// A sign-in method as the API lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub provider: Provider,
    /// A GitHub method's login.
    pub login: Option<String>,
}

/// A server with both providers' settings, codes that poll every second and the extra arguments
/// `args`; its public address is plain HTTP under `skald.test`.
pub fn server(args: &[&str]) -> Google {
    let mut all = vec![INTERVAL, "1"];
    all.extend_from_slice(args);
    Google::launch(Setup {
        github: true,
        args: &all,
        ..Setup::default()
    })
}

/// The headers of a request of the web session `cookie` from a page of `api`'s server.
pub fn from_page<'a>(api: &'a Api, cookie: &'a str) -> [(&'a str, &'a str); 2] {
    [("cookie", cookie), ("origin", api.public)]
}

/// The methods in `body`, a `200` answer to `GET /account/sign-in-methods`, in its order.
pub fn listed_in(body: &Value) -> Vec<Listed> {
    let items = body["methods"]
        .as_array()
        .unwrap_or_else(|| panic!("a list of methods: {body}"));
    items
        .iter()
        .map(|item| {
            let id = item["method"]
                .as_str()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| panic!("a method ID: {item}"));
            let (provider, login) = match (item["provider"].as_str(), item.get("github_login")) {
                (Some("github"), Some(Value::String(login))) => {
                    (Provider::GitHub, Some(login.clone()))
                }
                (Some("google"), None) => (Provider::Google, None),
                _ => panic!("a GitHub method with its login or a Google one without: {item}"),
            };
            Listed {
                id: id.to_owned(),
                provider,
                login,
            }
        })
        .collect()
}

/// The methods the session that `headers` carry lists; fails unless it lists them.
#[track_caller]
pub fn listed_with(api: &Api, headers: &[(&str, &str)]) -> Vec<Listed> {
    let response = api.call("get", METHODS, METHODS, headers);
    assert_eq!(response.status, 200, "{}", text(&response));
    listed_in(&json(&response))
}

/// The methods the web session `cookie` lists.
#[track_caller]
pub fn listed(api: &Api, cookie: &str) -> Vec<Listed> {
    listed_with(api, &[("cookie", cookie)])
}

/// The providers of `listed`, sorted.
pub fn sorted(listed: &[Listed]) -> Vec<Provider> {
    let mut providers: Vec<Provider> = listed.iter().map(|method| method.provider).collect();
    providers.sort();
    providers
}

/// The providers of an account with both methods, sorted.
pub const BOTH: [Provider; 2] = [Provider::GitHub, Provider::Google];

/// The ID of the one method of `provider` in `listed`.
#[track_caller]
pub fn id_of(listed: &[Listed], provider: Provider) -> String {
    let found: Vec<&Listed> = listed
        .iter()
        .filter(|method| method.provider == provider)
        .collect();
    assert_eq!(found.len(), 1, "one {provider:?} method in {listed:#?}");
    found[0].id.clone()
}

/// The path that removes the method `id`.
pub fn path_of(id: &str) -> String {
    format!("{METHODS}/{id}")
}

/// `DELETE /account/sign-in-methods/<id>` with `headers`.
pub fn remove_with(api: &Api, id: &str, headers: &[(&str, &str)]) -> Response {
    api.call("delete", &path_of(id), ONE, headers)
}

/// The web session `cookie` removes the method `id`, from a page of the server.
pub fn remove(api: &Api, cookie: &str, id: &str) -> Response {
    remove_with(api, id, &from_page(api, cookie))
}

/// Checks that `response` says the method is removed: a `204` without a body.
#[track_caller]
pub fn assert_removed(response: &Response) {
    assert_eq!(response.status, 204, "{}", text(response));
    assert!(response.body.is_empty(), "{}", text(response));
}

/// Checks that `response` refuses: a `4xx` with a problem details object.
#[track_caller]
pub fn assert_refused(response: &Response, what: &str) {
    assert!(
        (400..500).contains(&response.status),
        "{what}: {} {}",
        response.status,
        text(response)
    );
    assert!(
        response
            .content_type
            .starts_with("application/problem+json"),
        "{what}: {} {}",
        response.content_type,
        text(response)
    );
}

/// A way a test signs a person in.
#[derive(Clone, Copy, Debug)]
pub enum Via<'a> {
    Google(&'a Person),
    GitHub(&'a User),
}

/// A web session in a browser of its own.
pub struct Session<'a> {
    pub page: Page<'a>,
    pub who: Who,
    /// The `Cookie` header of the session.
    pub cookie: String,
}

/// An account made by a Google sign-in, with a GitHub method linked to it.
pub struct Both<'a> {
    pub person: Person,
    pub user: User,
    /// The web session of the Google sign-in.
    pub session: Session<'a>,
    /// The account's methods after the link.
    pub methods: Vec<Listed>,
}

impl Both<'_> {
    /// The ID of the account's method of `provider`.
    #[track_caller]
    pub fn id(&self, provider: Provider) -> String {
        id_of(&self.methods, provider)
    }
}

/// A test's server, with both providers' stand-ins, and its browser.
pub struct Stand {
    pub browser: Browser,
    pub google: Google,
}

impl Stand {
    /// A server as [`server`] starts it with `args`, and a browser.
    pub fn launch(args: &[&str]) -> Self {
        Stand {
            browser: Browser::launch(),
            google: server(args),
        }
    }

    pub fn api(&self) -> Api<'_> {
        Api::of_google(&self.google)
    }

    /// The Google person and the GitHub user of `name`, numbered `n` among a test's people.
    pub fn person(&self, n: u64, name: &str) -> (Person, User) {
        let google = &self.google;
        let person = google.person(&format!("3223{n:08}"), &format!("{name}@example.test"));
        let login = format!("{name}-standin");
        let user = google.github_user(3_223_000_000 + n, &login, &format!("{name}@example.org"));
        (person, user)
    }

    /// Signs in `via` on a new page of the browser.
    pub fn signed_in(&self, via: Via) -> Session<'_> {
        let page = self.browser.page();
        match via {
            Via::Google(person) => {
                self.google.sign_in(&page, person);
            }
            Via::GitHub(user) => self.google.github_sign_in(&page, user),
        }
        let who = self.google.account(&page);
        let cookie = cookie_of(&page);
        Session { page, who, cookie }
    }

    /// The account of `name`, numbered `n`: a Google sign-in, then a link of their GitHub user
    /// from the server's page of the account's sign-in methods.
    pub fn with_both(&self, n: u64, name: &str) -> Both<'_> {
        let (person, user) = self.person(n, name);
        let session = self.signed_in(Via::Google(&person));
        link_github(&self.google, &session.page, &user);
        let methods = listed(&self.api(), &session.cookie);
        assert_eq!(
            sorted(&methods),
            BOTH,
            "two methods after the link: {methods:#?}"
        );
        Both {
            person,
            user,
            session,
            methods,
        }
    }
}

/// Signs `page`'s browser in to GitHub as `user`, then links GitHub from the server's page of the
/// account's sign-in methods; gives the visit the link starts.
pub fn link_github(google: &Google, page: &Page, user: &User) -> Visit {
    common::sign_in::github_session(page, google.github(), user);
    page.goto(&format!("{}{METHODS_PAGE}", google.public));
    page.click_settled(LINK_GITHUB)
}

/// A machine of the account of `page`'s web session: the device session token it gets once the
/// person approves its code there.
pub fn machine_of(google: &Google, page: &Page) -> String {
    let machine = Machine::at(google.server.base());
    let grant = machine.authorize();
    machine.approve(page, &grant)
}
