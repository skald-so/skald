//! The harness of the web app's browser tests: the app's production build on one origin with a
//! `skald-server`, through a front in the browser's driver ([`Browser::spa_front`]), and a person
//! in headless Chrome. The client's tests include this file too.
//!
//! The web app, as these tests use it:
//! - without a web session, any page of it shows the sign-in page: a link or button for each
//!   provider the server has settings for, whose text names it, GitHub or Google, which starts that
//!   provider's sign-in and returns to the page; a note that says a first sign-in "creates an
//!   account" and, while sign-up is closed, that it does so "only" for the people the server lists;
//!   and the terminal box: its one textbox for the code and the button named "Continue", which
//!   opens the verification page `/device` with the code as `user_code`;
//! - with one, the cabinet shell of the UI kit: the side menu with the space switcher, a button
//!   whose name starts with "Space" that opens the list of the account's spaces, the account's
//!   personal space marked with the word "personal" there, and the account footer, with its menu
//!   button named "Account", which shows the username, the GitHub login of the account's GitHub
//!   method and the provider this session signed in with, "GitHub" or "Google". An account with no
//!   space gets an empty state in the side menu that says to sign in a machine with `skald`.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::browser::{Answer, Browser, Page};
pub use super::browser::{SIGN_IN_GITHUB, SIGN_IN_GOOGLE};

/// The terminal box's field for the code and its button.
pub const CODE_FIELD: &str = "role=textbox";
pub const CONTINUE: &str = r#"role=button[name="Continue"s]"#;
/// The menu button of the cabinet's account footer.
pub const ACCOUNT_MENU: &str = r#"role=button[name="Account"s]"#;

/// The server's session cookie: its value is a session token.
const SESSION_COOKIE: &str = "skald_session";

/// How long the web app may take to show what a test waits for.
pub const SHOW_WAIT: Duration = Duration::from_secs(30);

/// The JSON keys of what no answer to the web app holds: keys, certificates, roster revisions,
/// envelopes and session tokens.
const SECRET_KEYS: [&str; 19] = [
    "master",
    "key",
    "keys",
    "signing",
    "hpke",
    "cert",
    "certs",
    "certificate",
    "key_package",
    "key_packages",
    "revision",
    "revisions",
    "roster",
    "envelope",
    "envelopes",
    "ciphertext",
    "token",
    "access_token",
    "session_token",
];

/// A front of the web app: the app and the API on one origin.
pub struct Spa {
    pub port: u16,
    /// Its origin, as `http://skald.test:41234`.
    pub url: String,
}

impl Spa {
    /// Starts a front in `browser`'s driver; its origin is under `host`, such as `skald.test`.
    pub fn start(browser: &Browser, host: &str) -> Self {
        let port = browser.spa_front();
        Spa {
            port,
            url: format!("http://{host}:{port}"),
        }
    }

    /// Sends the front's API requests to the server at `target`, such as `http://127.0.0.1:41234`.
    pub fn serve(&self, browser: &Browser, target: &str) {
        browser.spa_target(self.port, target);
    }

    /// The address of the web app's page at `path`, such as `/`.
    pub fn at(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }
}

/// Waits for up to [`SHOW_WAIT`] until `shown` holds for `page`; fails naming `what` with the
/// page's text.
pub fn wait_until(page: &Page, what: &str, shown: impl Fn(&Page) -> bool) {
    let deadline = Instant::now() + SHOW_WAIT;
    while !shown(page) {
        assert!(
            Instant::now() < deadline,
            "{what} within {SHOW_WAIT:?}; the page shows: {:?}",
            page.text()
        );
        thread::sleep(Duration::from_millis(100));
    }
}

/// How many of the providers' sign-in buttons `page` shows: GitHub's and Google's.
pub fn sign_in_buttons(page: &Page) -> (usize, usize) {
    (page.count(SIGN_IN_GITHUB), page.count(SIGN_IN_GOOGLE))
}

/// Waits until `page` shows the sign-in page: a provider's button and no cabinet.
pub fn wait_for_sign_in_page(page: &Page) {
    wait_until(page, "the sign-in page", |page| {
        let (github, google) = sign_in_buttons(page);
        github + google > 0 && page.count(ACCOUNT_MENU) == 0
    });
}

/// Waits until `page` shows the cabinet whose footer names `username`, and no sign-in button.
pub fn wait_for_cabinet(page: &Page, username: &str) {
    wait_until(page, &format!("the cabinet of {username}"), |page| {
        page.count(ACCOUNT_MENU) == 1
            && sign_in_buttons(page) == (0, 0)
            && page.text().contains(username)
    });
}

/// The line of `page`'s text that says what a first sign-in creates; fails without one.
pub fn sign_up_note(page: &Page) -> String {
    let text = page.text();
    text.lines()
        .find(|line| {
            let line = line.to_lowercase();
            line.contains("creates an account") || line.contains("create an account")
        })
        .unwrap_or_else(|| panic!("no note on creating an account: {text:?}"))
        .to_owned()
}

/// Opens the space switcher of the cabinet in `page`, without leaving the page, and gives the text
/// of each space in its list, its accessible labels included; `None` without a switcher.
pub fn space_options(page: &Page) -> Option<Vec<String>> {
    let options = page.evaluate(
        r#"(async () => {
            const button = [...document.querySelectorAll("button")].find((button) =>
                /^Space\b/.test(button.getAttribute("aria-label") ?? button.textContent.trim()));
            if (!button) return null;
            if (button.getAttribute("aria-expanded") !== "true") button.click();
            const deadline = Date.now() + 10000;
            while (!document.querySelector("[role=option]") && Date.now() < deadline)
                await new Promise((resolve) => setTimeout(resolve, 50));
            return [...document.querySelectorAll("[role=option]")].map((option) =>
                [option, ...option.querySelectorAll("[aria-label], [title]")]
                    .map((part) => [part.getAttribute("aria-label"), part.getAttribute("title")])
                    .flat()
                    .filter(Boolean)
                    .concat([option.textContent])
                    .join(" ")
                    .trim());
        })()"#,
    );
    serde_json::from_value(options).expect("the space list")
}

/// Checks that none of `answers`, those the web app got, holds a key, a certificate, a roster
/// revision, an envelope or a session token: no JSON key of [`SECRET_KEYS`] and none of `tokens`,
/// the browser's session tokens.
#[track_caller]
pub fn assert_no_secrets(answers: &[Answer], tokens: &[String]) {
    assert!(!answers.is_empty(), "the web app got no answer");
    for answer in answers {
        let what = format!("{} {} {}", answer.method, answer.url, answer.status);
        for token in tokens {
            assert!(
                !answer.body.contains(token.as_str()),
                "{what}: a session token: {}",
                answer.body
            );
        }
        if let Ok(body) = serde_json::from_str::<Value>(&answer.body) {
            let mut found = Vec::new();
            secret_keys(&body, &mut found);
            assert_eq!(found, Vec::<String>::new(), "{what}: {}", answer.body);
        }
    }
}

/// The answers `page`'s browser recorded, checked by [`assert_no_secrets`] with the values of its
/// session cookies as its session tokens.
#[track_caller]
pub fn answers_without_secrets(page: &Page) -> Vec<Answer> {
    let answers = page.answers();
    let tokens: Vec<String> = page
        .cookies()
        .into_iter()
        .filter(|cookie| cookie.name == SESSION_COOKIE)
        .map(|cookie| cookie.value)
        .collect();
    assert_no_secrets(&answers, &tokens);
    answers
}

/// Adds each key of [`SECRET_KEYS`] in `value`, at any depth, to `found`.
fn secret_keys(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::Object(fields) => {
            for (key, field) in fields {
                if SECRET_KEYS.contains(&key.to_lowercase().as_str()) {
                    found.push(key.clone());
                }
                secret_keys(field, found);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| secret_keys(item, found)),
        _ => {}
    }
}
