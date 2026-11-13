//! The sign-in methods of an account on the server, against the GitHub stand-in: an account is
//! reached through its sign-in methods, and GitHub sign-in finds the account by its GitHub method
//! or creates one with it. Accounts made before the server had sign-in methods get their GitHub ID
//! as their GitHub method and keep their sessions, devices and master key. Every response is
//! checked against the OpenAPI spec in `api/openapi.json`.
//!
//! The database, as these tests read and change it: the table `sign_in_methods`, a row per method,
//! with
//! - `account_id`: the ID of the account it belongs to;
//! - `provider`: `github` for GitHub, an OIDC provider's issuer;
//! - `subject`: the subject the provider gives the person; for GitHub, the GitHub user's ID in
//!   decimal;
//! - `login`: a GitHub method's GitHub login, for display; `NULL` for other providers.

mod common;

use std::collections::BTreeMap;

use common::device_flow::{DeviceFlow, json, text};
use common::devices::{
    DEVICES, Device, Key, MASTER, assert_taken, b64, devices_of, master_of, random,
};
use common::sign_in::{Account, SESSION, SignIn, cookie_set};
use common::{Response, TestDb, get_url};
use skald_github_stand_in::User;
use skald_proto::AccountId;
use sqlx::{Connection, PgConnection};

/// The last migration of a server whose accounts had no sign-in methods: each account had its
/// GitHub ID and login.
const BEFORE_SIGN_IN_METHODS: i64 = 20261114000000;

/// An OIDC provider's issuer, for a method of a provider other than GitHub.
const GOOGLE: &str = "https://accounts.google.com";

/// A sign-in method as the database holds it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Method {
    account: String,
    provider: String,
    subject: String,
    login: Option<String>,
}

/// The GitHub method of `user` on the account `account`.
fn github(account: &str, user: &User) -> Method {
    Method {
        account: account.to_owned(),
        provider: "github".to_owned(),
        subject: user.id.to_string(),
        login: Some(user.login.clone()),
    }
}

/// `methods`, in the order [`methods`] gives them.
fn sorted(mut methods: Vec<Method>) -> Vec<Method> {
    methods.sort();
    methods
}

fn run<T>(work: impl Future<Output = Result<T, sqlx::Error>>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("start a tokio runtime")
        .block_on(work)
        .expect("query the test database")
}

/// Every sign-in method on `db`'s server, by account, provider and subject.
fn methods(db: &TestDb) -> Vec<Method> {
    let rows: Vec<(String, String, String, Option<String>)> = run(async {
        let mut conn = PgConnection::connect(&db.url()).await?;
        let rows = sqlx::query_as(
            "SELECT account_id::text, provider::text, subject::text, login::text
             FROM sign_in_methods",
        )
        .fetch_all(&mut conn)
        .await?;
        conn.close().await?;
        Ok(rows)
    });
    sorted(
        rows.into_iter()
            .map(|(account, provider, subject, login)| Method {
                account,
                provider,
                subject,
                login,
            })
            .collect(),
    )
}

/// The IDs of every account on `db`'s server, in order.
fn accounts(db: &TestDb) -> Vec<String> {
    let ids: Vec<(String,)> = run(async {
        let mut conn = PgConnection::connect(&db.url()).await?;
        let ids = sqlx::query_as("SELECT id::text FROM accounts ORDER BY id")
            .fetch_all(&mut conn)
            .await?;
        conn.close().await?;
        Ok(ids)
    });
    ids.into_iter().map(|(id,)| id).collect()
}

/// The account a request with the `Cookie` header `cookie` acts as; fails if it acts as none.
fn account_of_cookie(flow: &DeviceFlow, cookie: &str) -> Account {
    let response = flow.request("get", SESSION, &[("cookie", cookie)], None, true);
    assert_eq!(response.status, 200, "{}", text(&response));
    Account::of(&json(&response))
}

/// The `Cookie` header of the web session that the callback `callback` of a sign-in, started with
/// the answer `started`, gives.
fn finish(started: &Response, callback: &str) -> String {
    let signed_in = get_url(
        callback,
        &[("cookie", &cookie_set(started, "skald_sign_in"))],
    );
    assert_eq!(signed_in.status, 200, "{}", text(&signed_in));
    cookie_set(&signed_in, "skald_session")
}

/// Signs `user` in with GitHub and gives the session's account.
fn sign_in(flow: &DeviceFlow, user: &User) -> Account {
    account_of_cookie(flow, &flow.web_cookie(user))
}

/// A database with the schema of a server whose accounts had no sign-in methods.
fn database_before_methods() -> TestDb {
    let db = TestDb::create();
    run(async {
        let mut conn = PgConnection::connect(&db.url()).await?;
        skald_server::MIGRATOR
            .run_to(BEFORE_SIGN_IN_METHODS, &mut conn)
            .await
            .map_err(|err| sqlx::Error::Migrate(Box::new(err)))?;
        conn.close().await
    });
    db
}

/// `bytes` as a `bytea` literal.
fn bytea(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("'\\x{hex}'")
}

/// An account as a server before sign-in methods made it: a web session and a device session, and,
/// if it has a master key, a device with its certificate.
struct OldAccount {
    account: Account,
    user: User,
    /// The token of the web session, in the `skald_session` cookie.
    web: String,
    /// The token of the device session, sent as `Authorization: Bearer <token>`.
    device_session: String,
    master: Option<Key>,
    /// The device and its certificate, issued by the master key.
    device: Option<(Device, Vec<u8>)>,
}

impl OldAccount {
    /// Writes `user`'s account to `db` as the earlier server did, with a master key and a device if
    /// `with_device`.
    fn write(db: &TestDb, user: &User, with_device: bool) -> Self {
        let account = Account {
            id: AccountId::generate().to_string(),
            github_login: user.login.clone(),
        };
        let master = with_device.then(Key::generate);
        let device = master.as_ref().map(|master| {
            let device = Device::new("Old laptop");
            let cert = device.cert(&account, master);
            (device, cert)
        });
        let old = OldAccount {
            account,
            user: user.clone(),
            web: b64(&random()),
            device_session: b64(&random()),
            master,
            device,
        };
        let id = &old.account.id;
        let master = old
            .master
            .as_ref()
            .map_or("NULL".to_owned(), |master| bytea(&master.public()));
        let token = |token: &str| format!("sha256(convert_to('{token}', 'UTF8'))");
        let mut sql = format!(
            "INSERT INTO accounts (id, github_id, github_login, master)
             VALUES ('{id}', {github_id}, '{login}', {master});
             INSERT INTO sessions (token_hash, account_id, kind)
             VALUES ({web}, '{id}', 'web'), ({device}, '{id}', 'device');",
            github_id = user.id,
            login = user.login,
            web = token(&old.web),
            device = token(&old.device_session),
        );
        if let Some((device, cert)) = &old.device {
            sql.push_str(&format!(
                "INSERT INTO devices (id, account_id, kind, signing, hpke, name, cert)
                 VALUES ('{device_id}', '{id}', 'client', {signing}, {hpke},
                         convert_to('{name}', 'UTF8'), {cert});",
                device_id = device.id,
                signing = bytea(&device.signing.public()),
                hpke = bytea(&device.hpke),
                name = device.name,
                cert = bytea(cert),
            ));
        }
        db.execute(&sql);
        old
    }

    fn cookie(&self) -> String {
        format!("skald_session={}", self.web)
    }

    fn bearer(&self) -> String {
        format!("Bearer {}", self.device_session)
    }

    /// `<method> <path>` with the device session and the JSON `body` if given.
    fn send(
        &self,
        flow: &DeviceFlow,
        method: &str,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Response {
        let body = body.map(|body| serde_json::to_vec(body).expect("JSON"));
        let bearer = self.bearer();
        let headers = [("authorization", bearer.as_str())];
        flow.request(method, path, &headers, body.as_deref(), true)
    }

    /// The devices `GET /devices` gives the device session.
    fn devices(&self, flow: &DeviceFlow) -> BTreeMap<String, Option<Vec<u8>>> {
        devices_of(&self.send(flow, "get", DEVICES, None))
    }
}

#[test]
fn migrated_accounts_keep_sessions_devices_and_master_key_and_next_sign_in_reaches_them() {
    let db = database_before_methods();
    let ada = OldAccount::write(&db, &User::new(4_410_830_101, "ada-old-standin"), true);
    let bob = OldAccount::write(&db, &User::new(4_410_830_102, "bob-old-standin"), false);
    let flow = DeviceFlow::of(SignIn::on(db));
    let db = &flow.stand.db;
    let ids = sorted_ids(&[&ada.account.id, &bob.account.id]);

    // The migration gave each account its GitHub ID as its GitHub method.
    assert_eq!(
        methods(db),
        sorted(vec![
            github(&ada.account.id, &ada.user),
            github(&bob.account.id, &bob.user),
        ])
    );
    assert_eq!(accounts(db), ids);

    // Their web and device sessions still act as their accounts.
    for old in [&ada, &bob] {
        assert_eq!(account_of_cookie(&flow, &old.cookie()), old.account);
        assert_eq!(flow.account_of(&old.device_session), old.account);
    }

    // Their devices and master key stay.
    let (device, cert) = ada.device.as_ref().expect("ada's account has a device");
    let master = ada.master.as_ref().expect("ada's account has a master key");
    assert_eq!(
        ada.devices(&flow),
        BTreeMap::from([(device.id.to_string(), Some(cert.clone()))])
    );
    assert_eq!(
        master_of(&ada.send(&flow, "get", MASTER, None)),
        Some(master.public().to_vec())
    );
    assert_eq!(bob.devices(&flow), BTreeMap::new());
    assert_eq!(master_of(&bob.send(&flow, "get", MASTER, None)), None);

    // A device session registers a device as before.
    let desktop = Device::new("New desktop");
    assert_taken(&ada.send(&flow, "post", DEVICES, Some(&desktop.registration())));
    assert_eq!(
        ada.devices(&flow),
        BTreeMap::from([
            (device.id.to_string(), Some(cert.clone())),
            (desktop.id.to_string(), None),
        ])
    );

    // Each GitHub user's next sign-in reaches their account; Bob changed his login since.
    flow.stand.github.put_user(ada.user.clone());
    assert_eq!(sign_in(&flow, &ada.user), ada.account);
    let bob_renamed = User::new(bob.user.id, "bob-renamed-old-standin");
    flow.stand.github.put_user(bob_renamed.clone());
    let reached = sign_in(&flow, &bob_renamed);
    assert_eq!(reached.id, bob.account.id);
    assert_eq!(reached.github_login, bob_renamed.login);

    // No account or method was added, and the old sessions still act as their accounts.
    assert_eq!(accounts(db), ids);
    assert_eq!(
        methods(db),
        sorted(vec![
            github(&ada.account.id, &ada.user),
            github(&bob.account.id, &bob_renamed),
        ])
    );
    assert_eq!(account_of_cookie(&flow, &ada.cookie()), ada.account);
    assert_eq!(flow.account_of(&bob.device_session).id, bob.account.id);
}

#[test]
fn first_sign_in_creates_an_account_with_one_github_method() {
    let flow = DeviceFlow::start();
    let db = &flow.stand.db;
    let ada = flow.stand.user(4_410_830_201, "ada-first-standin");
    let bob = flow.stand.user(4_410_830_202, "bob-first-standin");
    assert_eq!(methods(db), Vec::new());

    let account = sign_in(&flow, &ada);
    assert_eq!(account.github_login, ada.login);
    assert_eq!(methods(db), vec![github(&account.id, &ada)]);
    assert_eq!(accounts(db), vec![account.id.clone()]);

    // Another GitHub user's first sign-in creates another account, with their method.
    let other = sign_in(&flow, &bob);
    assert_ne!(other.id, account.id);
    let both = sorted(vec![github(&account.id, &ada), github(&other.id, &bob)]);
    assert_eq!(methods(db), both);

    // The next sign-in reaches the account and adds no method.
    assert_eq!(sign_in(&flow, &ada), account);
    assert_eq!(sign_in(&flow, &bob), other);
    assert_eq!(methods(db), both);
    assert_eq!(accounts(db), sorted_ids(&[&account.id, &other.id]));
}

#[test]
fn two_first_sign_ins_of_one_github_user_at_once_create_one_account() {
    const ROUNDS: u64 = 5;
    let flow = DeviceFlow::start();
    let db = &flow.stand.db;
    let mut expected = Vec::new();

    for round in 0..ROUNDS {
        let user = flow.stand.user(
            4_410_830_300 + round,
            &format!("ada-at-once-{round}-standin"),
        );
        // Two browsers of the person, each held at GitHub's redirect back to the server.
        let held = [flow.stand.to_callback(&user), flow.stand.to_callback(&user)];
        let barrier = std::sync::Barrier::new(held.len());
        let cookies: Vec<String> = std::thread::scope(|scope| {
            let callbacks: Vec<_> = held
                .iter()
                .map(|(started, callback)| {
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        finish(started, callback)
                    })
                })
                .collect();
            callbacks
                .into_iter()
                .map(|done| done.join().expect("a sign-in"))
                .collect()
        });
        let reached: Vec<Account> = cookies
            .iter()
            .map(|cookie| account_of_cookie(&flow, cookie))
            .collect();
        assert_eq!(reached[0], reached[1], "round {round}");
        assert_eq!(reached[0].github_login, user.login, "round {round}");
        expected.push(github(&reached[0].id, &user));
    }

    let ids: Vec<&String> = expected.iter().map(|method| &method.account).collect();
    assert_eq!(accounts(db), sorted_ids(&ids));
    assert_eq!(methods(db), sorted(expected));
}

#[test]
fn login_change_updates_the_method_login_and_keeps_the_account() {
    let flow = DeviceFlow::start();
    let db = &flow.stand.db;
    let grace = flow.stand.user(4_410_830_401, "grace-method-standin");
    let first = flow.web_cookie(&grace);
    let account = account_of_cookie(&flow, &first);

    let renamed = User::new(grace.id, "grace-renamed-method-standin");
    flow.stand.github.put_user(renamed.clone());
    let reached = sign_in(&flow, &renamed);
    let updated = Account {
        id: account.id.clone(),
        github_login: renamed.login.clone(),
    };
    assert_eq!(reached, updated);
    assert_eq!(account_of_cookie(&flow, &first), updated);
    assert_eq!(methods(db), vec![github(&account.id, &renamed)]);

    // Another GitHub user who takes the old login gets an account and a method of their own.
    let newcomer = flow.stand.user(4_410_830_402, &grace.login);
    let theirs = sign_in(&flow, &newcomer);
    assert_ne!(theirs.id, account.id);
    assert_eq!(
        methods(db),
        sorted(vec![
            github(&account.id, &renamed),
            github(&theirs.id, &newcomer),
        ])
    );
}

#[test]
fn github_sign_in_reaches_only_the_account_that_holds_the_github_method() {
    let flow = DeviceFlow::start();
    let db = &flow.stand.db;
    let ada = flow.stand.user(4_410_830_501, "ada-holder-standin");
    let bob = flow.stand.user(4_410_830_502, "bob-holder-standin");
    let account = sign_in(&flow, &ada);
    let bobs = sign_in(&flow, &bob);

    // The account's GitHub method is replaced by a method of another provider: no account holds
    // Ada's GitHub method any more.
    db.execute(&format!(
        "UPDATE sign_in_methods SET provider = '{GOOGLE}', subject = '108204371559281736420',
                login = NULL
         WHERE account_id = '{}'",
        account.id
    ));
    let google = Method {
        account: account.id.clone(),
        provider: GOOGLE.to_owned(),
        subject: "108204371559281736420".to_owned(),
        login: None,
    };

    // Ada's next GitHub sign-in is a first one: it creates an account with her GitHub method.
    let next = sign_in(&flow, &ada);
    assert_ne!(next.id, account.id);
    assert_ne!(next.id, bobs.id);
    assert_eq!(next.github_login, ada.login);
    assert_eq!(
        methods(db),
        sorted(vec![google, github(&bobs.id, &bob), github(&next.id, &ada)])
    );
    assert_eq!(accounts(db), sorted_ids(&[&account.id, &bobs.id, &next.id]));
    assert_eq!(sign_in(&flow, &ada), next);
    assert_eq!(sign_in(&flow, &bob), bobs);
}

/// `ids`, owned and in order, as [`accounts`] gives them.
fn sorted_ids(ids: &[&String]) -> Vec<String> {
    let mut ids: Vec<String> = ids.iter().map(|id| (*id).clone()).collect();
    ids.sort();
    ids
}
