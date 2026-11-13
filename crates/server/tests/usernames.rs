//! Account usernames on the server, against the GitHub stand-in: each account has its own username,
//! unique on the server, which any of its sessions changes; a GitHub sign-up makes it from the
//! GitHub login, and the GitHub login is only its default. Every response to a request is checked
//! against the OpenAPI spec in `api/openapi.json`.
//!
//! The API, each request stating the protocol version:
//! - `GET /session`: `{"account": <ID>, "username": <username>, "github_login": <login>}`, with
//!   `github_login` only while the account has a GitHub sign-in method.
//! - `PUT /account/username` of `{"username": <username>}`: sets the session's account's username.
//!   Set: `200` or `204`. Refused, with a problem details object: `400` for an invalid request or
//!   username, `401` without a valid session, `409` for a reserved or taken username.
//!
//! A username has the form of a `name` in an agent address (`docs/protocol/README.md`, "Agent
//! address"); `skald`, `admin`, `support` and `root` are reserved. A username made from a text, such
//! as a GitHub login: lowercase; each run of characters a `name` does not allow becomes `-`; cut to
//! a `name`'s length; `-` trimmed at both ends; a default when nothing is left. When that is taken
//! or reserved, the first free of `-2`, `-3` and on is appended, within a `name`'s length.

mod common;

use std::collections::BTreeSet;
use std::sync::Barrier;
use std::thread;

use common::device_flow::{DeviceFlow, json, text};
use common::devices::{assert_refused, b64, machine, random};
use common::sign_in::{SESSION, SignIn, cookie_set};
use common::{Response, TestDb, get_url};
use serde_json::{Value, json};
use skald_github_stand_in::User;
use skald_proto::AccountId;
use sqlx::{Connection, PgConnection};

/// Sets the session's account's username: `PUT`.
const USERNAME: &str = "/account/username";

/// The usernames no account takes.
const RESERVED: [&str; 4] = ["skald", "admin", "support", "root"];

/// The most characters of a `name`.
const NAME_MAX: usize = 63;

/// The last migration of a server whose accounts had no username.
const BEFORE_USERNAMES: i64 = 20261115000000;

/// An OIDC provider's issuer, for a method of a provider other than GitHub.
const GOOGLE: &str = "https://accounts.google.com";

/// A session's account as `GET /session` gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Who {
    account: String,
    username: String,
    github_login: Option<String>,
}

impl Who {
    /// The account in `response`, a `200` answer to `GET /session`.
    fn of(response: &Response) -> Self {
        assert_eq!(response.status, 200, "{}", text(response));
        let body = json(response);
        let account = body["account"].as_str().expect("an account ID").to_owned();
        let username = body["username"]
            .as_str()
            .unwrap_or_else(|| panic!("no username in {body}"))
            .to_owned();
        let github_login = match body.get("github_login") {
            None => None,
            Some(Value::String(login)) => Some(login.clone()),
            Some(other) => panic!("github_login {other} in {body}"),
        };
        Who {
            account,
            username,
            github_login,
        }
    }
}

/// Whether `text` has the form of a `name` in an agent address.
fn is_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    let alnum = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    (1..=NAME_MAX).contains(&bytes.len())
        && alnum(&bytes[0])
        && alnum(&bytes[bytes.len() - 1])
        && bytes.iter().all(|byte| alnum(byte) || *byte == b'-')
}

/// Checks that `username` is one an account can have: a `name`, not reserved.
fn assert_valid(username: &str) {
    assert!(is_name(username), "not a name: {username:?}");
    assert!(!RESERVED.contains(&username), "reserved: {username:?}");
}

/// The account of the web session in the `Cookie` header `cookie`.
fn of_cookie(flow: &DeviceFlow, cookie: &str) -> Who {
    Who::of(&flow.request("get", SESSION, &[("cookie", cookie)], None, true))
}

/// The account of the device session of the session token `token`.
fn of_token(flow: &DeviceFlow, token: &str) -> Who {
    Who::of(&flow.session(Some(&format!("Bearer {token}"))))
}

/// Signs the GitHub user `id`, `login`, in with GitHub; gives the web session's `Cookie` header and
/// its account.
fn sign_in(flow: &DeviceFlow, id: u64, login: &str) -> (String, Who) {
    let user = flow.stand.user(id, login);
    let cookie = flow.web_cookie(&user);
    let who = of_cookie(flow, &cookie);
    assert_eq!(who.github_login.as_deref(), Some(login), "{who:?}");
    (cookie, who)
}

/// The username a GitHub user's sign-up gives.
fn signed_up(flow: &DeviceFlow, id: u64, login: &str) -> String {
    let (_, who) = sign_in(flow, id, login);
    assert_valid(&who.username);
    who.username
}

/// `PUT /account/username` of the JSON `body` with the request headers `headers`.
fn put(flow: &DeviceFlow, headers: &[(&str, &str)], body: &[u8]) -> Response {
    flow.request("put", USERNAME, headers, Some(body), true)
}

/// The headers of a browser's request with the web session `cookie`, from the server's own page.
fn web(flow: &DeviceFlow, cookie: &str) -> Vec<(String, String)> {
    vec![
        ("cookie".to_owned(), cookie.to_owned()),
        ("origin".to_owned(), flow.stand.public.clone()),
    ]
}

/// The headers of a request with the device session `token`.
fn device(token: &str) -> Vec<(String, String)> {
    vec![("authorization".to_owned(), format!("Bearer {token}"))]
}

/// Asks to set `username` with the request headers `headers`.
fn set(flow: &DeviceFlow, headers: &[(String, String)], username: &str) -> Response {
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    let body = serde_json::to_vec(&json!({ "username": username })).expect("JSON");
    put(flow, &headers, &body)
}

/// Checks that the server set the username `response` answers.
fn assert_set(response: &Response) {
    assert!(
        matches!(response.status, 200 | 204),
        "set: {}",
        text(response)
    );
}

/// Runs `job` on each of `inputs`, each in a thread of its own, all at once; gives the results in
/// the order of `inputs`.
fn at_once<I: Sync, T: Send>(inputs: &[I], job: impl Fn(&I) -> T + Sync) -> Vec<T> {
    let ready = Barrier::new(inputs.len());
    let (ready, job) = (&ready, &job);
    thread::scope(|scope| {
        let mut running = Vec::new();
        for input in inputs {
            running.push(scope.spawn(move || {
                ready.wait();
                job(input)
            }));
        }
        running
            .into_iter()
            .map(|run| run.join().expect("a thread at once"))
            .collect()
    })
}

/// The text of the verification page that the web session `cookie` gets.
fn verification_page(flow: &DeviceFlow, cookie: &str) -> String {
    let grant = flow.authorize();
    let path = grant
        .verification_uri
        .strip_prefix(&flow.stand.public)
        .unwrap_or_else(|| panic!("{grant:?} is under {}", flow.stand.public));
    let url = format!("{}{path}", flow.stand.server.base());
    let page = get_url(&url, &[("cookie", cookie)]);
    assert_eq!(page.status, 200, "{}", text(&page));
    String::from_utf8_lossy(&page.body).into_owned()
}

#[test]
fn github_sign_up_takes_the_login_in_lowercase() {
    let flow = DeviceFlow::start();
    assert_eq!(
        signed_up(&flow, 4_410_840_101, "ada-lower-standin"),
        "ada-lower-standin"
    );
    assert_eq!(
        signed_up(&flow, 4_410_840_102, "Bob-Upper-Standin"),
        "bob-upper-standin"
    );
    assert_eq!(signed_up(&flow, 4_410_840_103, "C0DE42"), "c0de42");

    // The session still gives the GitHub login as GitHub has it, to a device session too.
    let user = flow.stand.user(4_410_840_102, "Bob-Upper-Standin");
    let machine = machine(&flow, &user);
    let who = of_token(&flow, &machine.token);
    assert_eq!(who.username, "bob-upper-standin");
    assert_eq!(who.github_login.as_deref(), Some("Bob-Upper-Standin"));
}

#[test]
fn login_that_is_no_valid_name_still_gives_a_valid_username() {
    let flow = DeviceFlow::start();
    // An Enterprise Managed User's login: the user's name, `_` and the enterprise's short code.
    assert_eq!(
        signed_up(&flow, 4_410_840_201, "Ada-Lovelace_acme"),
        "ada-lovelace-acme"
    );
    // Each run of characters a `name` does not allow becomes one `-`; `-` is trimmed at both ends.
    assert_eq!(
        signed_up(&flow, 4_410_840_202, "_grace__hopper._corp_"),
        "grace-hopper-corp"
    );
    // Cut to a `name`'s length, then trimmed.
    let long = format!("{}_yz{}", "X".repeat(62), "w".repeat(10));
    assert_eq!(signed_up(&flow, 4_410_840_203, &long), "x".repeat(62));
    // Nothing is left: a default, a valid username all the same, and another for the next one.
    let first = signed_up(&flow, 4_410_840_204, "___");
    let second = signed_up(&flow, 4_410_840_205, "_");
    assert_ne!(first, second);
}

#[test]
fn taken_or_reserved_default_gets_the_first_free_number() {
    let flow = DeviceFlow::start();

    // Each of one login's sign-ups takes the next number.
    let base = "gap-standin";
    assert_eq!(signed_up(&flow, 4_410_840_301, base), base);
    let (second, who) = sign_in(&flow, 4_410_840_302, base);
    assert_eq!(who.username, "gap-standin-2");
    assert_eq!(signed_up(&flow, 4_410_840_303, base), "gap-standin-3");
    // Once `-2` is free again, it is the first free one.
    assert_set(&set(&flow, &web(&flow, &second), "bee-standin"));
    assert_eq!(signed_up(&flow, 4_410_840_304, base), "gap-standin-2");
    assert_eq!(
        signed_up(&flow, 4_410_840_305, "Gap-Standin"),
        "gap-standin-4"
    );

    // A username an account chose is taken for a login that makes it.
    assert_eq!(
        signed_up(&flow, 4_410_840_306, "bee-standin"),
        "bee-standin-2"
    );

    // A reserved username goes to no account; the first free number does.
    for (at, reserved) in RESERVED.iter().enumerate() {
        let login = reserved.to_uppercase();
        let id = 4_410_840_310 + at as u64;
        assert_eq!(signed_up(&flow, id, &login), format!("{reserved}-2"));
    }
    assert_eq!(signed_up(&flow, 4_410_840_320, "skald"), "skald-3");

    // The number fits within a `name`'s length.
    let long = "x".repeat(NAME_MAX);
    assert_eq!(signed_up(&flow, 4_410_840_330, &long), long);
    let numbered = format!("{}-2", "x".repeat(NAME_MAX - 2));
    assert_eq!(signed_up(&flow, 4_410_840_331, &long), numbered);
}

#[test]
fn two_sign_ups_with_one_login_at_once_get_different_usernames() {
    const ROUNDS: u64 = 5;
    let flow = DeviceFlow::start();

    for round in 0..ROUNDS {
        // Two GitHub users whose logins make one username, such as one who took the login of the
        // other since the other's last sign-in.
        let login = format!("twin-{round}-standin");
        let users = [
            flow.stand.user(4_410_840_400 + 2 * round, &login),
            flow.stand
                .user(4_410_840_401 + 2 * round, &login.to_uppercase()),
        ];
        let held: Vec<_> = users
            .iter()
            .map(|user| flow.stand.to_callback(user))
            .collect();
        // The browsers come back from GitHub at once.
        let cookies = at_once(&held, |(started, callback)| {
            let jar = cookie_set(started, "skald_sign_in");
            let back = get_url(callback, &[("cookie", jar.as_str())]);
            assert_eq!(back.status, 200, "{}", text(&back));
            cookie_set(&back, "skald_session")
        });
        let usernames: BTreeSet<String> = cookies
            .iter()
            .map(|cookie| of_cookie(&flow, cookie).username)
            .collect();
        assert_eq!(
            usernames,
            BTreeSet::from([login.clone(), format!("{login}-2")]),
            "round {round}"
        );
    }
}

/// An account as a server before usernames made it: a GitHub method and a web session.
struct OldAccount {
    id: String,
    user: User,
    /// The `Cookie` header of its web session.
    cookie: String,
}

impl OldAccount {
    /// Writes the account of the GitHub user `id`, `login`, to `db` as the earlier server did.
    fn write(db: &TestDb, id: u64, login: &str) -> Self {
        let account = AccountId::generate().to_string();
        let token = b64(&random());
        db.execute(&format!(
            "INSERT INTO accounts (id) VALUES ('{account}');
             INSERT INTO sign_in_methods (provider, subject, account_id, login)
             VALUES ('github', '{id}', '{account}', '{login}');
             INSERT INTO sessions (token_hash, account_id, kind)
             VALUES (sha256(convert_to('{token}', 'UTF8')), '{account}', 'web');"
        ));
        OldAccount {
            id: account,
            user: User::new(id, login),
            cookie: format!("skald_session={token}"),
        }
    }
}

/// A database with the schema of a server whose accounts had no username.
fn database_before_usernames() -> TestDb {
    let db = TestDb::create();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("start a tokio runtime")
        .block_on(async {
            let mut conn = PgConnection::connect(&db.url()).await?;
            skald_server::MIGRATOR
                .run_to(BEFORE_USERNAMES, &mut conn)
                .await
                .map_err(|err| sqlx::Error::Migrate(Box::new(err)))?;
            conn.close().await
        })
        .expect("migrate the test database");
    db
}

#[test]
fn migration_gives_existing_accounts_their_logins_numbered_when_two_share_one() {
    let db = database_before_usernames();
    let ada = OldAccount::write(&db, 4_410_840_501, "Ada-Migrated-Standin");
    // An Enterprise Managed User, and a user whose login makes the same username.
    let emu = OldAccount::write(&db, 4_410_840_502, "grace_acme");
    let grace = OldAccount::write(&db, 4_410_840_503, "grace-acme");
    // Two accounts whose GitHub methods share a login: one user took it since the other's sign-in.
    let first = OldAccount::write(&db, 4_410_840_504, "dup-standin");
    let second = OldAccount::write(&db, 4_410_840_505, "dup-standin");
    let admin = OldAccount::write(&db, 4_410_840_506, "admin");
    let flow = DeviceFlow::of(SignIn::on(db));

    let username = |old: &OldAccount| {
        let who = of_cookie(&flow, &old.cookie);
        assert_eq!(who.account, old.id);
        assert_eq!(who.github_login.as_deref(), Some(old.user.login.as_str()));
        who.username
    };
    assert_eq!(username(&ada), "ada-migrated-standin");
    assert_eq!(
        BTreeSet::from([username(&emu), username(&grace)]),
        BTreeSet::from(["grace-acme".to_owned(), "grace-acme-2".to_owned()])
    );
    assert_eq!(
        BTreeSet::from([username(&first), username(&second)]),
        BTreeSet::from(["dup-standin".to_owned(), "dup-standin-2".to_owned()])
    );
    assert_eq!(username(&admin), "admin-2");

    // The next sign-in of a migrated account keeps the username the migration gave it.
    let kept = username(&second);
    flow.stand.github.put_user(second.user.clone());
    let cookie = flow.web_cookie(&second.user);
    let who = of_cookie(&flow, &cookie);
    assert_eq!(who.account, second.id);
    assert_eq!(who.username, kept);

    // A new sign-up takes the first free number after the migrated ones.
    assert_eq!(
        signed_up(&flow, 4_410_840_507, "dup-standin"),
        "dup-standin-3"
    );
    assert_eq!(
        signed_up(&flow, 4_410_840_508, "ada-migrated-standin"),
        "ada-migrated-standin-2"
    );
}

#[test]
fn login_change_keeps_the_username() {
    let flow = DeviceFlow::start();
    let (cookie, account) = sign_in(&flow, 4_410_840_601, "grace-name-standin");
    assert_eq!(account.username, "grace-name-standin");

    let renamed = User::new(4_410_840_601, "Grace-Renamed-Standin");
    flow.stand.github.put_user(renamed.clone());
    let who = of_cookie(&flow, &flow.web_cookie(&renamed));
    assert_eq!(who.account, account.account);
    assert_eq!(who.username, "grace-name-standin");
    assert_eq!(who.github_login.as_deref(), Some("Grace-Renamed-Standin"));
    assert_eq!(of_cookie(&flow, &cookie), who);

    // A username the user chose stays at the next login change too.
    assert_set(&set(&flow, &web(&flow, &cookie), "grace-chosen-standin"));
    let again = User::new(4_410_840_601, "grace-again-standin");
    flow.stand.github.put_user(again.clone());
    let who = of_cookie(&flow, &flow.web_cookie(&again));
    assert_eq!(who.username, "grace-chosen-standin");
    assert_eq!(who.github_login.as_deref(), Some("grace-again-standin"));

    // Another GitHub user who takes a login whose username an account holds gets a number.
    assert_eq!(
        signed_up(&flow, 4_410_840_602, "grace-chosen-standin"),
        "grace-chosen-standin-2"
    );
}

#[test]
fn web_and_device_sessions_set_a_free_username_and_the_verification_page_names_it() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(4_410_840_701, "ada-set-standin");
    let machine = machine(&flow, &ada);
    let account = of_cookie(&flow, &machine.cookie).account;

    // The web session sets one.
    assert_set(&set(
        &flow,
        &web(&flow, &machine.cookie),
        "ada-from-the-web",
    ));
    for who in [
        of_cookie(&flow, &machine.cookie),
        of_token(&flow, &machine.token),
    ] {
        assert_eq!(who.account, account);
        assert_eq!(who.username, "ada-from-the-web");
        assert_eq!(who.github_login.as_deref(), Some("ada-set-standin"));
    }
    let page = verification_page(&flow, &machine.cookie);
    assert!(page.contains("ada-from-the-web"), "{page}");

    // The device session sets another; the web session sees it.
    assert_set(&set(&flow, &device(&machine.token), "ada-from-a-machine"));
    for who in [
        of_cookie(&flow, &machine.cookie),
        of_token(&flow, &machine.token),
    ] {
        assert_eq!(who.username, "ada-from-a-machine");
        assert_eq!(who.github_login.as_deref(), Some("ada-set-standin"));
    }
    let page = verification_page(&flow, &machine.cookie);
    assert!(page.contains("ada-from-a-machine"), "{page}");

    // A username that equals another person's GitHub login is free while no account holds it: it
    // proves no GitHub identity.
    let (bob, _) = sign_in(&flow, 4_410_840_702, "bob-set-standin");
    assert_set(&set(&flow, &web(&flow, &bob), "robert-standin"));
    assert_set(&set(&flow, &device(&machine.token), "bob-set-standin"));
    assert_eq!(of_token(&flow, &machine.token).username, "bob-set-standin");
    assert_eq!(of_cookie(&flow, &bob).username, "robert-standin");
}

#[test]
fn every_form_of_a_name_is_a_username() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(4_410_840_801, "ada-forms-standin");
    let machine = machine(&flow, &ada);
    let longest = format!("a{}", "-b".repeat(31));
    assert_eq!(longest.len(), NAME_MAX);
    for username in ["q", "7", "42", "a--b", "skald-2", "admins", &longest] {
        assert_set(&set(&flow, &device(&machine.token), username));
        assert_eq!(of_token(&flow, &machine.token).username, username);
    }
}

#[test]
fn invalid_reserved_or_taken_username_is_refused_and_the_old_one_stays() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(4_410_840_901, "ada-refused-standin");
    let machine = machine(&flow, &ada);
    let (_, bob) = sign_in(&flow, 4_410_840_902, "bob-refused-standin");
    let headers = [web(&flow, &machine.cookie), device(&machine.token)];
    let stays = |flow: &DeviceFlow| {
        assert_eq!(
            of_token(flow, &machine.token).username,
            "ada-refused-standin"
        );
        assert_eq!(
            of_cookie(flow, &machine.cookie).username,
            "ada-refused-standin"
        );
    };

    let invalid = [
        String::new(),
        "Ada".to_owned(),
        "ADA-REFUSED".to_owned(),
        "-ada".to_owned(),
        "ada-".to_owned(),
        "-".to_owned(),
        "ada_lovelace".to_owned(),
        "ada.lovelace".to_owned(),
        "ada lovelace".to_owned(),
        " ada".to_owned(),
        "ada/claude".to_owned(),
        "ada@skald".to_owned(),
        "ünal".to_owned(),
        "x".repeat(NAME_MAX + 1),
    ];
    for headers in &headers {
        for username in &invalid {
            assert_refused(&set(&flow, headers, username), 400);
            stays(&flow);
        }
        for reserved in RESERVED {
            assert_refused(&set(&flow, headers, reserved), 409);
            stays(&flow);
        }
        assert_refused(&set(&flow, headers, &bob.username), 409);
        stays(&flow);
    }

    // A body that is no username.
    let bearer = format!("Bearer {}", machine.token);
    let auth = [("authorization", bearer.as_str())];
    for body in [
        &b"not JSON"[..],
        br#"{}"#,
        br#"{"username": 42}"#,
        br#"{"username": null}"#,
        br#"["ada-new"]"#,
    ] {
        assert_refused(&put(&flow, &auth, body), 400);
        stays(&flow);
    }

    // The other account keeps its username.
    let who = of_cookie(
        &flow,
        &sign_in(&flow, 4_410_840_902, "bob-refused-standin").0,
    );
    assert_eq!(who.username, bob.username);
}

#[test]
fn setting_a_username_without_a_session_is_refused() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(4_410_841_001, "ada-nosession-standin");
    let machine = machine(&flow, &ada);
    let body = serde_json::to_vec(&json!({ "username": "ada-no-session" })).expect("JSON");
    let origin = flow.stand.public.clone();
    for headers in [
        vec![],
        vec![("origin", origin.as_str())],
        vec![
            ("cookie", "skald_session=unknown"),
            ("origin", origin.as_str()),
        ],
        vec![("authorization", "Bearer unknown")],
        vec![("authorization", "Basic YWRhOnNlY3JldA==")],
    ] {
        assert_refused(&put(&flow, &headers, &body), 401);
    }
    // A device session's token is no web session, and a web session's token no device session.
    let token = machine.token.clone();
    let cookie_token = machine
        .cookie
        .trim_start_matches("skald_session=")
        .to_owned();
    let as_cookie = format!("skald_session={token}");
    let as_bearer = format!("Bearer {cookie_token}");
    assert_refused(
        &put(&flow, &[("cookie", &as_cookie), ("origin", &origin)], &body),
        401,
    );
    assert_refused(&put(&flow, &[("authorization", &as_bearer)], &body), 401);
    assert_eq!(of_token(&flow, &token).username, "ada-nosession-standin");
}

#[test]
fn two_accounts_setting_one_username_at_once_get_it_once() {
    const ROUNDS: usize = 5;
    let flow = DeviceFlow::start();
    let ada = machine(&flow, &flow.stand.user(4_410_841_101, "ada-race-standin"));
    let bob = machine(&flow, &flow.stand.user(4_410_841_102, "bob-race-standin"));
    let machines = [&ada, &bob];

    for round in 0..ROUNDS {
        let wanted = format!("race-{round}-standin");
        let answers = at_once(&machines, |machine| {
            set(&flow, &device(&machine.token), &wanted)
        });
        let won: Vec<usize> = (0..machines.len())
            .filter(|at| matches!(answers[*at].status, 200 | 204))
            .collect();
        assert_eq!(
            won.len(),
            1,
            "round {round}: {:?}",
            answers.iter().map(text).collect::<Vec<_>>()
        );
        let lost = 1 - won[0];
        assert_refused(&answers[lost], 409);
        assert_eq!(of_token(&flow, &machines[won[0]].token).username, wanted);
        assert_ne!(of_token(&flow, &machines[lost].token).username, wanted);
    }
}

#[test]
fn account_without_a_github_method_gives_its_username_and_no_github_login() {
    let flow = DeviceFlow::start();
    let ada = flow.stand.user(4_410_841_201, "ada-google-standin");
    let machine = machine(&flow, &ada);
    let account = of_token(&flow, &machine.token).account;
    assert_set(&set(&flow, &device(&machine.token), "ada-of-google"));

    // The account's GitHub method is replaced by a method of another provider.
    flow.stand.db.execute(&format!(
        "UPDATE sign_in_methods SET provider = '{GOOGLE}', subject = '108204371559281736421',
                login = NULL
         WHERE account_id = '{account}'"
    ));

    let expected = Who {
        account: account.clone(),
        username: "ada-of-google".to_owned(),
        github_login: None,
    };
    assert_eq!(of_cookie(&flow, &machine.cookie), expected);
    assert_eq!(of_token(&flow, &machine.token), expected);
    let page = verification_page(&flow, &machine.cookie);
    assert!(page.contains("ada-of-google"), "{page}");

    // Its sessions still set its username.
    assert_set(&set(
        &flow,
        &web(&flow, &machine.cookie),
        "ada-still-google",
    ));
    let who = of_token(&flow, &machine.token);
    assert_eq!(who.username, "ada-still-google");
    assert_eq!(who.github_login, None);
    let page = verification_page(&flow, &machine.cookie);
    assert!(page.contains("ada-still-google"), "{page}");

    // A GitHub sign-up whose login makes a username the account holds gets a number.
    assert_set(&set(&flow, &device(&machine.token), "ada-google-standin"));
    assert_eq!(
        signed_up(&flow, 4_410_841_202, "Ada-Google-Standin"),
        "ada-google-standin-2"
    );
}
