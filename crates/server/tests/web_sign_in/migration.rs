//! The migration that records each web session's sign-in method ends the web sessions from before,
//! whose method it cannot tell, and keeps the device sessions.

use crate::common::browser::{Browser, Cookie};
use crate::common::device_flow::{DeviceFlow, Machine, json, text};
use crate::common::devices::{b64, random};
use crate::common::google::HOST;
use crate::common::sign_in::{SESSION, SignIn};
use crate::common::spa::{Spa, wait_for_sign_in_page};
use crate::common::{Server, TestDb};

/// The last migration of a server whose web sessions had no sign-in method.
const BEFORE_METHODS: i64 = 20261210000000;

/// The session cookie's name.
const SESSION_COOKIE: &str = "skald_session";

/// An account as a server before the sign-in methods of web sessions left it: its GitHub method, a
/// web session and a device session.
struct Before {
    account: String,
    /// The web session's token, in the session cookie.
    web: String,
    /// The device session's token, sent as `Authorization: Bearer <token>`.
    device: String,
}

impl Before {
    /// Writes the account of GitHub user `id` with `login` and its sessions to `db`.
    fn write(db: &TestDb, id: u64, login: &str) -> Self {
        let before = Before {
            account: skald_proto::AccountId::generate().to_string(),
            web: b64(&random()),
            device: b64(&random()),
        };
        let hash = |token: &str| format!("sha256(convert_to('{token}', 'UTF8'))");
        db.execute(&format!(
            "INSERT INTO accounts (id, username) VALUES ('{account}', '{username}');
             INSERT INTO sign_in_methods (provider, subject, account_id, login)
             VALUES ('github', '{id}', '{account}', '{login}');
             INSERT INTO sessions (token_hash, account_id, kind)
             VALUES ({web}, '{account}', 'web'), ({device}, '{account}', 'device');",
            account = before.account,
            username = login.to_lowercase(),
            web = hash(&before.web),
            device = hash(&before.device),
        ));
        before
    }

    /// The `Cookie` header of its web session.
    fn cookie(&self) -> String {
        format!("{SESSION_COOKIE}={}", self.web)
    }
}

/// A database whose migrations end before the sign-in methods of web sessions, holding the account
/// of GitHub user `id` with `login`.
fn database_before(id: u64, login: &str) -> (TestDb, Before) {
    let db = TestDb::create();
    db.migrate_to(BEFORE_METHODS);
    let before = Before::write(&db, id, login);
    (db, before)
}

/// `GET /session` of the device session `token` straight to `server`: fails unless it is
/// `before`'s account.
#[track_caller]
fn assert_device_session_works(server: &Server, before: &Before) {
    let response = Machine::at(server.base()).session(Some(&format!("Bearer {}", before.device)));
    assert_eq!(
        response.status,
        200,
        "the device session: {}",
        text(&response)
    );
    assert_eq!(json(&response)["account"], before.account.as_str());
}

/// After the migration, the web session from before is refused, and the device session still acts
/// as its account.
#[test]
fn migration_ends_web_sessions_from_before_and_keeps_device_sessions() {
    let (db, before) = database_before(3_220_500_001, "Old-Web-Standin");
    let flow = DeviceFlow::of(SignIn::on(db));

    let web = flow.request("get", SESSION, &[("cookie", &before.cookie())], None, true);
    assert_eq!(
        web.status,
        401,
        "the web session from before: {}",
        text(&web)
    );
    assert_device_session_works(&flow.stand.server, &before);
}

/// In the browser, after the migration, the web app gets the sign-in page with the web session's
/// cookie from before.
#[test]
fn web_app_with_a_web_session_from_before_the_migration_shows_the_sign_in_page() {
    let (db, before) = database_before(3_220_500_011, "Older-Web-Standin");
    let browser = Browser::launch();
    let spa = Spa::start(&browser, HOST);
    let stand = SignIn::behind_on(db, &spa.url, &[]);
    spa.serve(&browser, stand.server.base());

    let page = browser.page();
    page.add_cookies(&[Cookie {
        name: SESSION_COOKIE.to_owned(),
        value: before.web.clone(),
        domain: HOST.to_owned(),
        path: "/".to_owned(),
        expires: -1.0,
        http_only: true,
        secure: false,
        same_site: "Lax".to_owned(),
    }]);
    page.goto(&spa.at("/"));
    wait_for_sign_in_page(&page);
    assert_device_session_works(&stand.server, &before);
}
