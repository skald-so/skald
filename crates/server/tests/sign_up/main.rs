//! Acceptance tests of closed sign-up: while a server's sign-up is closed, a sign-in that would
//! create an account creates one only for a listed identity; existing accounts sign in and link
//! methods as before; a refused sign-up creates nothing and says why, in the browser and to the
//! device code it was for. `skald-server` runs against the GitHub stand-in and the OIDC stand-in,
//! which plays Google, with the person's part in headless Chrome. Responses are checked against the
//! OpenAPI spec in `api/openapi.json`.
//!
//! The server, as these tests use it, beside `common/google.rs` and `common/device_flow.rs`:
//! - `--sign-up <open|closed>`, or `SKALD_SIGN_UP`: `open` by default. While it is open, the server
//!   logs a warning at start, a line with `warning` that names sign-up; an invalid value stops it
//!   at start, naming the setting;
//! - the lists, comma-separated, each also from its environment variable: `--sign-up-github-logins`
//!   (`SKALD_SIGN_UP_GITHUB_LOGINS`), GitHub logins; `--sign-up-google-emails`
//!   (`SKALD_SIGN_UP_GOOGLE_EMAILS`), Google emails, which count only with `email_verified` true;
//!   `--sign-up-google-domains` (`SKALD_SIGN_UP_GOOGLE_DOMAINS`), the `hd` of Google accounts, never
//!   an email's suffix. Entries compare with identities without case;
//! - a refused sign-up: the callback answers `403` with a page that names the problem type
//!   [`REFUSED`] and says sign-up is closed; for a sign-in started on the verification page with a
//!   code, polls for that code answer `400` in the `OAuthError` format, its `error` `access_denied`
//!   or a code of the server's own, and `error_description` saying sign-up is closed.

#[path = "../common/mod.rs"]
mod common;

mod device;
mod existing;
mod lists;
mod spec;
mod start;

use std::thread;
use std::time::{Duration, Instant};

use common::Server;
use common::accounts::Who;
use common::browser::{Page, Visit};
use common::device_flow::INTERVAL;
use common::google::{
    CLIENT_ID, CLIENT_SECRET, GITHUB_START, Google, SECRET_ENV, Setup, text as body_text,
};
use common::sign_in::{
    CLIENT_SECRET as GITHUB_CLIENT_SECRET, SECRET_ENV as GITHUB_SECRET_ENV, github_args,
    github_rows, github_session,
};
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;

/// The settings of sign-up and its lists.
pub const SIGN_UP: &str = "--sign-up";
pub const GITHUB_LOGINS: &str = "--sign-up-github-logins";
pub const GOOGLE_EMAILS: &str = "--sign-up-google-emails";
pub const GOOGLE_DOMAINS: &str = "--sign-up-google-domains";
/// The same settings in the environment.
pub const SIGN_UP_ENV: &str = "SKALD_SIGN_UP";
pub const GITHUB_LOGINS_ENV: &str = "SKALD_SIGN_UP_GITHUB_LOGINS";
pub const GOOGLE_EMAILS_ENV: &str = "SKALD_SIGN_UP_GOOGLE_EMAILS";
pub const GOOGLE_DOMAINS_ENV: &str = "SKALD_SIGN_UP_GOOGLE_DOMAINS";

/// The problem type of a refused sign-up.
pub const REFUSED: &str = "https://skald.so/problems/sign-up-closed";

/// The lists of a closed server; an empty one is not given.
#[derive(Clone, Copy, Default)]
pub struct Lists<'a> {
    pub logins: &'a str,
    pub emails: &'a str,
    pub domains: &'a str,
}

impl Lists<'_> {
    /// The arguments of a closed sign-up with these lists.
    pub fn args(&self) -> Vec<&str> {
        let mut args = vec![SIGN_UP, "closed"];
        for (flag, list) in [
            (GITHUB_LOGINS, self.logins),
            (GOOGLE_EMAILS, self.emails),
            (GOOGLE_DOMAINS, self.domains),
        ] {
            if !list.is_empty() {
                args.extend([flag, list]);
            }
        }
        args
    }
}

/// A server with GitHub's and Google's settings whose sign-up is closed with `lists`, its device
/// codes polling every second.
pub fn closed(lists: Lists) -> Google {
    let mut args = lists.args();
    args.extend([INTERVAL, "1"]);
    Google::launch(Setup {
        github: true,
        args: &args,
        ..Setup::default()
    })
}

/// A server with GitHub's and Google's settings and no sign-up setting, its device codes polling
/// every second.
pub fn default_server() -> Google {
    Google::launch(Setup {
        github: true,
        args: &[INTERVAL, "1"],
        ..Setup::default()
    })
}

/// Stops `google`'s server and starts it again on its database, port and stand-ins with the extra
/// arguments `args` and the environment variables `envs`, as an operator changes its settings.
pub fn restarted(google: Google, args: &[&str], envs: &[(&str, &str)]) -> Google {
    let Google {
        db,
        provider,
        github,
        server,
        public,
    } = google;
    let listen = server
        .base()
        .strip_prefix("http://")
        .expect("an http address")
        .to_owned();
    drop(server);
    let server = {
        let github = github.as_ref().expect("the server has GitHub's settings");
        let mut all = vec![
            "--public-url",
            public.as_str(),
            "--google-client-id",
            CLIENT_ID,
            "--google-issuer",
            provider.issuer(),
        ];
        all.extend(github_args(github));
        all.extend([INTERVAL, "1"]);
        all.extend_from_slice(args);
        let mut all_envs = vec![
            (SECRET_ENV, CLIENT_SECRET),
            (GITHUB_SECRET_ENV, GITHUB_CLIENT_SECRET),
        ];
        all_envs.extend_from_slice(envs);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match Server::try_launch(&db, &listen, &all, &all_envs) {
                Ok(server) => break server,
                Err(taken) if Instant::now() < deadline => {
                    eprintln!("the old server's port is not free yet: {taken:?}");
                    thread::sleep(Duration::from_millis(200));
                }
                Err(taken) => panic!("cannot start the server again on {listen}: {taken:?}"),
            }
        }
    };
    Google {
        db,
        provider,
        github,
        server,
        public,
    }
}

/// A person at the OIDC stand-in with `email`, verified if `verified`, and the `hd` claim `domain`,
/// if any.
pub fn person(
    google: &Google,
    subject: &str,
    email: &str,
    verified: bool,
    domain: Option<&str>,
) -> Person {
    let person = Person {
        email_verified: verified,
        hosted_domain: domain.map(str::to_owned),
        ..Person::new(subject, email)
    };
    google.provider.put_person(person.clone());
    person
}

/// A GitHub user without a public email at the GitHub stand-in.
pub fn github_user(google: &Google, id: u64, login: &str) -> User {
    let user = User::new(id, login);
    google.github().put_user(user.clone());
    user
}

/// Signs `page`'s browser in to GitHub as `user`, then starts a GitHub sign-in on the server;
/// gives where it ended.
pub fn github_visit(google: &Google, page: &Page, user: &User) -> Visit {
    github_session(page, google.github(), user);
    page.goto(&format!("{}{GITHUB_START}", google.public))
}

/// The page's whole HTML, its source as the browser has it.
pub fn html(page: &Page) -> String {
    page.evaluate("document.documentElement.outerHTML")
        .as_str()
        .expect("the page's HTML")
        .to_owned()
}

/// Whether `text` says that sign-up is closed.
pub fn says_closed(text: &str) -> bool {
    let text = text.to_lowercase();
    (text.contains("sign-up") || text.contains("sign up")) && text.contains("closed")
}

/// Checks that the sign-in that ended in `visit` at `page` was refused as a sign-up: a `403` page
/// that names the problem type and says sign-up is closed, and no session; `what` names the case.
#[track_caller]
pub fn assert_refused(google: &Google, page: &Page, visit: &Visit, what: &str) {
    assert_eq!(
        visit.status,
        Some(403),
        "{what}: {visit:?}: {}",
        page.text()
    );
    let source = html(page);
    assert!(
        source.contains(REFUSED),
        "{what}: the page names {REFUSED}: {source}"
    );
    assert!(
        says_closed(&page.text()),
        "{what}: the page says sign-up is closed: {}",
        page.text()
    );
    google.assert_signed_out(page);
}

/// Checks that GitHub's `user` has no account: no row holds their ID or login.
#[track_caller]
pub fn assert_no_github_account(google: &Google, user: &User, what: &str) {
    let rows = github_rows(&google.db, user);
    assert!(rows.is_empty(), "{what}: {rows:?}");
}

/// Signs `user` in with GitHub at `page`; fails unless the session's account has their login. Gives
/// the account.
pub fn github_signed_up(google: &Google, page: &Page, user: &User) -> Who {
    let visit = github_visit(google, page, user);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    let who = google.account(page);
    assert_eq!(
        who.github_login.as_deref(),
        Some(user.login.as_str()),
        "{who:?}"
    );
    who
}

/// The body of `response` as text, with its status.
pub fn shown(response: &common::Response) -> String {
    format!("{} {}", response.status, body_text(response))
}
