//! The web app on its harness against a server with the providers and settings a test names, and
//! the checks of what the web app's browser got.

use serde_json::Value;
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;

use crate::common::Response;
use crate::common::browser::{Browser, Page, Visit};
use crate::common::google::{Google, HOST, Setup};
use crate::common::sign_in::{github_session, page_get};
use crate::common::spa::{
    SIGN_IN_GITHUB, SIGN_IN_GOOGLE, Spa, answers_without_secrets, wait_for_sign_in_page,
};
use crate::common::spec::{operation_violations, spec};

/// The providers a server has settings for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Providers {
    GitHub,
    Google,
    Both,
}

/// The web app on the harness: its front, the server behind it with the stand-ins of its
/// providers, and the person's browser.
pub struct Web {
    pub browser: Browser,
    pub spa: Spa,
    pub stand: Google,
}

impl Web {
    /// A server with both providers' settings and open sign-up.
    pub fn start() -> Self {
        Self::launch(Providers::Both, &[])
    }

    /// A server with the settings of `providers` and the extra arguments `args`; its public address
    /// is the front's.
    pub fn launch(providers: Providers, args: &[&str]) -> Self {
        let browser = Browser::launch();
        let spa = Spa::start(&browser, HOST);
        let stand = Google::launch(Setup {
            public: Some(&spa.url),
            github: providers != Providers::Google,
            no_google: providers == Providers::GitHub,
            args,
            ..Setup::default()
        });
        spa.serve(&browser, stand.server.base());
        Web {
            browser,
            spa,
            stand,
        }
    }

    /// Adds a GitHub user to the GitHub stand-in and gives them.
    pub fn github_user(&self, id: u64, login: &str) -> User {
        let user = User::new(id, login);
        self.stand.github().put_user(user.clone());
        user
    }

    /// A new browser, which records the answers it gets.
    pub fn page(&self) -> Page<'_> {
        let page = self.browser.page();
        page.record_answers();
        page
    }

    /// Signs `page`'s browser in to GitHub as `user`, opens the web app's page at `path` and, on its
    /// sign-in page, clicks GitHub: one click. Gives the click's visit, its redirects and
    /// refreshes followed.
    pub fn sign_in_with_github(&self, page: &Page, user: &User, path: &str) -> Visit {
        github_session(page, self.stand.github(), user);
        self.click_on_sign_in_page(page, path, SIGN_IN_GITHUB)
    }

    /// As [`Web::sign_in_with_github`], signing in to Google as `person` and clicking Google.
    pub fn sign_in_with_google(&self, page: &Page, person: &Person, path: &str) -> Visit {
        self.stand.provider_session(page, person);
        self.click_on_sign_in_page(page, path, SIGN_IN_GOOGLE)
    }

    /// Opens the web app's page at `path` in `page`, waits for its sign-in page and clicks the
    /// button `button` finds there.
    fn click_on_sign_in_page(&self, page: &Page, path: &str, button: &str) -> Visit {
        page.goto(&self.spa.at(path));
        wait_for_sign_in_page(page);
        page.click_settled(button)
    }

    /// `GET /session` from a page of the web app in `page`'s browser, which leaves the web app: the
    /// session's account; fails without one.
    pub fn session(&self, page: &Page) -> Value {
        let (status, body) = page_get(page, &self.spa.url, "/session");
        assert_eq!(status, 200, "GET /session: {body}");
        body
    }
}

/// Checks the answers `page`'s browser recorded: each conforms to the OpenAPI spec, and none holds
/// a key, a certificate, a roster revision, an envelope or a session token of its cookies.
#[track_caller]
pub fn assert_answers(page: &Page) {
    let answers = answers_without_secrets(page);
    let spec = spec();
    for answer in &answers {
        let path = path_of(&answer.url);
        let mut headers = answer.headers.clone();
        let response = Response {
            status: answer.status,
            content_type: headers.remove("content-type").unwrap_or_default(),
            headers: headers.into_iter().collect(),
            body: answer.body.clone().into_bytes(),
        };
        let method = answer.method.to_lowercase();
        let violations = operation_violations(&spec, &method, &spec_path(&spec, &path), &response);
        assert_eq!(violations, Vec::<String>::new(), "{} {path}", answer.method);
    }
}

/// The path of `url`, without its origin and query.
fn path_of(url: &str) -> String {
    let parsed = url::Url::parse(url).unwrap_or_else(|err| panic!("{url}: {err}"));
    parsed.path().to_owned()
}

/// The path of `spec` that the request path `path` matches, a `{parameter}` standing for any
/// segment; `path` itself when none does.
fn spec_path(spec: &Value, path: &str) -> String {
    let wanted: Vec<&str> = path.split('/').collect();
    let paths = spec["paths"].as_object().expect("paths in the spec");
    if paths.contains_key(path) {
        return path.to_owned();
    }
    for candidate in paths.keys() {
        let parts: Vec<&str> = candidate.split('/').collect();
        let fits = parts.len() == wanted.len()
            && parts
                .iter()
                .zip(&wanted)
                .all(|(part, got)| part == got || part.starts_with('{'));
        if fits {
            return candidate.clone();
        }
    }
    path.to_owned()
}
