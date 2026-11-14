//! The account's username in the CLI: `skald sign-in` names the account by its username, which the
//! server gives, never by its GitHub login, and `skald set-username` changes it through the API.
//! Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, behind
//! the recording relay, with a test playing the person on the verification page in headless
//! Chrome. Each machine keeps its state in a directory of its own and its secrets in a file
//! there, never in the system's key storage.
//!
//! The CLI, as these tests use it:
//! - `skald set-username <username>`: sets the username of the account of the machine's session,
//!   on the server of its local state, and names it; a username the server refuses fails the run
//!   with the server's reason;
//! - `skald sign-in`, signed in or run again, and its error when another account approves the code
//!   name the account by its username.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;

use std::fs;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use browser::Browser;
use cloud::Cloud;
use machine::{DEVICES, FIRST_DEVICE, Finished, Machine, NO, SERVER, SIGN_IN, SIGN_OUT, YES};
use serde_json::{Value, json};
use skald_github_stand_in::User;
use skald_proto::{Cbor, Cert};

/// The command that sets the account's username.
const SET_USERNAME: &str = "set-username";

/// The usernames no account takes.
const RESERVED: [&str; 4] = ["skald", "admin", "support", "root"];

/// The most characters of a `name`, and so of a username.
const NAME_MAX: usize = 63;

/// The local state's file in the state directory.
const STATE_FILE: &str = "state.sqlite3";

/// The local state as clients before usernames wrote it, at schema version 3: its session keeps the
/// account's GitHub login. Only its tables and columns, which are what a later client reads.
const STATE_BEFORE_USERNAMES: &str = "
    CREATE TABLE session (
        id INTEGER PRIMARY KEY, server TEXT NOT NULL, account TEXT, github_login TEXT
    ) STRICT;
    CREATE TABLE device (
        id INTEGER PRIMARY KEY, device TEXT NOT NULL, name TEXT NOT NULL,
        registered INTEGER NOT NULL DEFAULT 0, link TEXT NOT NULL DEFAULT 'new', master BLOB,
        cert BLOB, account TEXT
    ) STRICT;
    PRAGMA user_version = 3;
";

/// An OIDC provider's issuer, for a sign-in method of a provider other than GitHub.
const GOOGLE: &str = "https://accounts.google.com";

/// A person with an account and the account's first machine, signed in.
struct Person {
    user: User,
    machine: Machine,
    /// The machine's session token.
    token: String,
    /// What the machine's first sign-in printed.
    signed_in: Finished,
}

/// A server whose codes live for 2 minutes and poll every second, and a browser for the person.
fn setup() -> (Cloud, Browser) {
    (Cloud::start(120, 1), Browser::launch())
}

/// Signs the first machine of GitHub user `id` with `login` in, on the host `host`.
fn person(cloud: &Cloud, browser: &Browser, id: u64, login: &str, host: &str) -> Person {
    let user = cloud.user(id, login);
    let before = cloud.tokens().len();
    let (machine, signed_in) = cloud.first_machine(&browser.page(), &user, host);
    assert!(signed_in.success, "{signed_in:?}");
    let tokens = cloud.tokens();
    assert_eq!(tokens.len(), before + 1, "one new session token");
    let token = tokens[before].clone();
    Person {
        user,
        machine,
        token,
        signed_in,
    }
}

/// The username of the account of the session `token`, as the server gives it.
fn username_of(cloud: &Cloud, token: &str) -> String {
    let (status, body) = cloud.session_of(token);
    assert_eq!(status, 200, "GET /session: {body}");
    body["username"]
        .as_str()
        .unwrap_or_else(|| panic!("a username: {body}"))
        .to_owned()
}

/// `PUT /account/username` with `username`, sent by the test itself with the session `token`:
/// the status and the body.
fn put_username(cloud: &Cloud, token: &str, username: &str) -> (u16, Value) {
    let agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let body = json!({ "username": username }).to_string();
    let mut response = agent
        .put(format!("{}/account/username", cloud.url))
        .header(
            skald_proto::VERSION_HEADER,
            skald_proto::PROTOCOL_VERSION.to_string(),
        )
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .send(body.as_str())
        .unwrap_or_else(|err| panic!("PUT /account/username: {err}"));
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .unwrap_or_else(|err| panic!("read PUT /account/username: {err}"));
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// Runs `skald set-username <username>` on `machine`.
fn set_username(machine: &Machine, username: &str) -> Finished {
    machine.start(&[SET_USERNAME, username]).finish()
}

/// Runs `skald sign-in` again on `machine`, which has a session on `cloud`; fails unless it
/// succeeds with that session, asking for no new code.
fn sign_in_again(cloud: &Cloud, machine: &Machine, first: &str) -> Finished {
    let grants = cloud.grants().len();
    let again = machine
        .start(&[SIGN_IN, SERVER, &cloud.url, FIRST_DEVICE, first])
        .finish();
    assert!(again.success, "{again:?}");
    assert_eq!(cloud.grants().len(), grants, "a new code: {again:?}");
    again
}

/// Fails unless `finished` names `username` and not the GitHub login `login`.
fn assert_names(finished: &Finished, username: &str, login: &str) {
    let shown = finished.shown();
    assert!(shown.contains(username), "{username} is not named: {shown}");
    assert!(
        !shown.contains(login),
        "the GitHub login {login} is named: {shown}"
    );
}

/// Runs `skald set-username <wanted>` on `person`'s first machine; fails unless it succeeds naming
/// `wanted`, the server gives `wanted` as the account's username, and the machine's next sign-in
/// names it. Gives what that sign-in printed.
fn assert_sets(cloud: &Cloud, person: &Person, wanted: &str) -> Finished {
    let set = set_username(&person.machine, wanted);
    assert!(set.success, "{set:?}");
    assert!(set.shown().contains(wanted), "{set:?}");
    assert_eq!(username_of(cloud, &person.token), wanted);
    let again = sign_in_again(cloud, &person.machine, YES);
    assert_names(&again, wanted, &person.user.login);
    again
}

/// Whether one of `machine`'s SQLite files, its local state, holds `text`.
fn state_holds(machine: &Machine, text: &str) -> bool {
    let state = machine.sqlite_files();
    assert!(!state.is_empty(), "no local state in {:?}", machine.root());
    machine
        .files_holding(text)
        .iter()
        .any(|file| state.contains(file))
}

/// Sign-in names the account by the username the server made from the GitHub login, not by the
/// login, both signed in and run again; the local state keeps the username, not the login.
#[test]
fn sign_in_names_the_username_and_not_the_github_login() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_001,
        "Ada_Lovelace_Standin",
        "ada-workstation",
    );
    let username = "ada-lovelace-standin";
    assert_eq!(username_of(&cloud, &ada.token), username);

    assert_names(&ada.signed_in, username, &ada.user.login);
    assert!(state_holds(&ada.machine, username), "the state");
    assert!(
        !state_holds(&ada.machine, &ada.user.login),
        "the state keeps the GitHub login"
    );
    let again = sign_in_again(&cloud, &ada.machine, YES);
    assert_names(&again, username, &ada.user.login);
}

/// After the GitHub login changes, sign-in still names the account's username: on a new machine,
/// whose sign-in brings the new login to the server, and on the machine signed in before.
#[test]
fn sign_in_names_the_same_username_after_the_github_login_changes() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_101,
        "Ada_Renames_Standin",
        "ada-workstation",
    );
    let username = "ada-renames-standin";
    let renamed = cloud.user(ada.user.id, "Ada_Renamed_Standin");

    let second = Machine::new().with_host_name("ada-laptop");
    let signed_in = cloud.signed_in(&browser.page(), &second, &renamed, &[FIRST_DEVICE, NO]);

    assert!(signed_in.success, "{signed_in:?}");
    let token = cloud.last_token();
    assert_eq!(cloud.login_of(&token), renamed.login, "the login changed");
    assert_eq!(username_of(&cloud, &token), username);
    assert_names(&signed_in, username, &renamed.login);
    assert!(
        !signed_in.shown().contains(&ada.user.login),
        "{signed_in:?}"
    );
    let again = sign_in_again(&cloud, &ada.machine, YES);
    assert_names(&again, username, &renamed.login);
    assert!(!again.shown().contains(&ada.user.login), "{again:?}");
}

/// `skald set-username` sets a free username on the server and names it; the next sign-in names
/// it, on this machine and on a new one.
#[test]
fn set_username_sets_a_free_username_and_sign_in_names_it() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_201,
        "Ada_Sets_Standin",
        "ada-workstation",
    );
    let wanted = "ada-of-skald";

    let again = assert_sets(&cloud, &ada, wanted);

    assert!(!again.shown().contains("ada-sets-standin"), "{again:?}");

    let second = Machine::new().with_host_name("ada-laptop");
    let signed_in = cloud.signed_in(&browser.page(), &second, &ada.user, &[FIRST_DEVICE, NO]);
    assert!(signed_in.success, "{signed_in:?}");
    assert_names(&signed_in, wanted, &ada.user.login);
}

/// A machine of one account, signed out and then approved by the person of another, fails naming
/// the approver's account by the username it set, not by its GitHub login.
#[test]
fn other_account_error_names_the_username_set_with_the_cli() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_301,
        "Ada_Approves_Standin",
        "ada-workstation",
    );
    let bob = person(
        &cloud,
        &browser,
        4_120_669_302,
        "Bob_Device_Standin",
        "bob-workstation",
    );
    let wanted = "ada-the-approver";
    let set = set_username(&ada.machine, wanted);
    assert!(set.success, "{set:?}");
    let out = bob.machine.start(&[SIGN_OUT]).finish();
    assert!(out.success, "{out:?}");

    let refused = cloud.signed_in(
        &browser.page(),
        &bob.machine,
        &ada.user,
        &[FIRST_DEVICE, YES],
    );

    assert!(!refused.success, "{refused:?}");
    assert_names(&refused, wanted, &ada.user.login);
    assert!(
        !refused.shown().contains("ada-approves-standin"),
        "{refused:?}"
    );
}

/// An invalid, reserved or taken username is refused: `skald set-username` fails with the reason
/// the server gives for it, and the account keeps its old username.
#[test]
fn invalid_reserved_or_taken_username_is_refused_with_the_servers_reason_and_the_old_one_stays() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_401,
        "Ada_Refused_Standin",
        "ada-workstation",
    );
    let bob = person(
        &cloud,
        &browser,
        4_120_669_402,
        "Bob_Taken_Standin",
        "bob-workstation",
    );
    let old = "ada-refused-standin";
    let taken = username_of(&cloud, &bob.token);
    let too_long = "a".repeat(NAME_MAX + 1);
    let mut refused: Vec<(&str, u16)> = vec![
        ("Ada-Refused", 400),
        ("ada_lovelace", 400),
        ("ada.lovelace", 400),
        ("ada-", 400),
        (&too_long, 400),
        (&taken, 409),
    ];
    refused.extend(RESERVED.iter().map(|reserved| (*reserved, 409)));

    for (username, status) in refused {
        let run = set_username(&ada.machine, username);

        assert!(!run.success, "{username:?} was taken: {run:?}");
        assert_eq!(username_of(&cloud, &ada.token), old, "after {username:?}");
        let (answered, body) = put_username(&cloud, &ada.token, username);
        assert_eq!(answered, status, "the server on {username:?}: {body}");
        let reason = body["detail"]
            .as_str()
            .unwrap_or_else(|| panic!("the server's reason: {body}"));
        assert!(!reason.is_empty(), "the server's reason: {body}");
        assert!(
            run.shown().contains(reason),
            "{username:?}: the server's reason {reason:?} is not given: {run:?}"
        );
    }
    assert_eq!(username_of(&cloud, &bob.token), taken);
    let again = sign_in_again(&cloud, &ada.machine, YES);
    assert_names(&again, old, &ada.user.login);
}

/// Replaces the local state of `person`'s machine with the one a client before usernames wrote for
/// the same session and device: the session with the account's GitHub login, the device certified
/// with the certificate and the master key the server has. The secrets stay.
fn write_state_before_usernames(cloud: &Cloud, person: &Person) {
    let (status, session) = cloud.session_of(&person.token);
    assert_eq!(status, 200, "GET /session: {session}");
    let account = session["account"]
        .as_str()
        .expect("an account ID")
        .to_owned();
    let master = cloud
        .master(&person.token)
        .expect("the account's master key");
    let (status, listed) = cloud.get_as("/devices", &person.token);
    assert_eq!(status, 200, "GET /devices: {listed}");
    let devices = listed["devices"].as_array().expect("a device list");
    assert_eq!(devices.len(), 1, "{listed}");
    let device = devices[0]["device"]
        .as_str()
        .expect("a device ID")
        .to_owned();
    let cert = URL_SAFE_NO_PAD
        .decode(devices[0]["cert"].as_str().expect("a certificate"))
        .expect("a base64url certificate");
    let name = Cert::from_cbor(&cert)
        .expect("a certificate")
        .body()
        .name
        .clone();

    for file in person.machine.sqlite_files() {
        fs::remove_file(&file).expect("remove the local state");
    }
    let db = rusqlite::Connection::open(person.machine.state_dir().join(STATE_FILE))
        .expect("create the old local state");
    db.execute_batch(STATE_BEFORE_USERNAMES)
        .expect("the old schema");
    db.execute(
        "INSERT INTO session (id, server, account, github_login) VALUES (1, ?1, ?2, ?3)",
        rusqlite::params![cloud.url, account, person.user.login],
    )
    .expect("the old session");
    db.execute(
        "INSERT INTO device (id, device, name, registered, link, master, cert, account)
         VALUES (1, ?1, ?2, 1, 'certified', ?3, ?4, ?5)",
        rusqlite::params![device, name, master.to_vec(), cert, account],
    )
    .expect("the old device");
}

/// Local state from a client before usernames, which kept the GitHub login, migrates: the next
/// sign-in keeps its session and its device, and names the username the server gives.
#[test]
fn local_state_from_before_keeps_its_session_and_names_the_username() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_501,
        "Ada_Before_Standin",
        "ada-workstation",
    );
    let username = "ada-from-before";
    let (status, body) = put_username(&cloud, &ada.token, username);
    assert!(matches!(status, 200 | 204), "PUT /account/username: {body}");
    write_state_before_usernames(&cloud, &ada);
    let mark = cloud.relay.mark();

    let again = sign_in_again(&cloud, &ada.machine, NO);

    let bearer = format!("\nauthorization: bearer {}", ada.token.to_lowercase());
    assert!(
        cloud
            .relay
            .sent_since(mark)
            .iter()
            .any(|request| request.head.contains(&bearer)),
        "its session token was not sent: {again:?}"
    );
    assert_eq!(cloud.tokens().len(), 1, "a new session");
    assert_eq!(ada.machine.files_holding(&ada.token).len(), 1, "the token");
    let listed = ada.machine.start(&[DEVICES]).finish();
    assert!(listed.success, "{listed:?}");
    let (_, devices) = cloud.get_as("/devices", &ada.token);
    let device = devices["devices"][0]["device"]
        .as_str()
        .expect("a device ID");
    assert!(listed.stdout.contains(device), "{listed:?}");
    assert_eq!(devices["devices"].as_array().map(Vec::len), Some(1));

    assert_names(&again, username, &ada.user.login);
    assert!(state_holds(&ada.machine, username), "the state");
}

/// An account whose GitHub sign-in method an operator replaced with one of another provider has no
/// GitHub login: sign-in names it by its username, and `skald set-username` changes that.
#[test]
fn account_with_a_sign_in_method_of_another_provider_is_named_by_its_username() {
    let (cloud, browser) = setup();
    let ada = person(
        &cloud,
        &browser,
        4_120_669_601,
        "Ada_Google_Standin",
        "ada-workstation",
    );
    let username = "ada-google-standin";
    let account = cloud.account_of(&ada.token);
    cloud.execute(&format!(
        "UPDATE sign_in_methods SET provider = '{GOOGLE}', subject = '108204371559281736422',
                login = NULL
         WHERE account_id = '{account}'"
    ));
    let (_, session) = cloud.session_of(&ada.token);
    assert_eq!(session.get("github_login"), None, "{session}");

    let again = sign_in_again(&cloud, &ada.machine, YES);
    assert_names(&again, username, &ada.user.login);

    let again = assert_sets(&cloud, &ada, "ada-still-google");
    assert!(!again.shown().contains(username), "{again:?}");
}
