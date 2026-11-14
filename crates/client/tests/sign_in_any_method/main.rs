//! Acceptance tests of `skald sign-in` with any sign-in method: the machine signs in by device flow
//! whichever provider its person signs in with on the verification page. Against a `skald-server`
//! with Google sign-in through the OIDC stand-in, which plays Google, alone or with GitHub sign-in
//! through the GitHub stand-in, on PostgreSQL, behind the recording relay, with a test playing the
//! person on the verification page in headless Chrome. Each machine keeps its state in a directory
//! of its own and its secrets in a file there, never in the system's key storage.
//!
//! The server and the CLI, as these tests use them, beside the sign-in tests':
//! - the verification page without a session offers sign-in with each provider the server has
//!   settings for, as a link or button whose text names it, GitHub or Google;
//! - an account made by a Google sign-in has the username made from the email's local part, and
//!   `skald sign-in` names it;
//! - `GET /account/sign-in-methods` gives a session's account's methods, and the page
//!   `/sign-in-methods` links Google to a web session's account with its button "Link Google".

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;

use std::path::PathBuf;

use browser::{Browser, Page};
use cloud::{Cloud, Device, GIVE_MASTER, Providers, REGISTER, ids};
use machine::{FIRST_DEVICE, Finished, Machine, SERVER, SIGN_IN, SIGN_OUT, YES};
use serde_json::Value;
use skald_github_stand_in::User;
use skald_oidc_stand_in::Person;
use skald_proto::{AccountId, Cert, DeviceKind};

/// The page of the account's sign-in methods, and its button that links Google.
const METHODS_PAGE: &str = "/sign-in-methods";
const LINK_GOOGLE: &str = "role=button[name=/link.*google/i]";
/// The account's sign-in methods in the API.
const METHODS: &str = "/account/sign-in-methods";

/// A server with the settings of `providers` whose codes live for 2 minutes and poll every second,
/// and a browser for the person.
fn setup(providers: Providers) -> (Cloud, Browser) {
    (Cloud::with(providers, 120, 1), Browser::launch())
}

/// The account's first machine, signed in.
struct First {
    machine: Machine,
    /// What its sign-in printed.
    finished: Finished,
    /// Its session token.
    token: String,
    account: AccountId,
    /// The account's master public key.
    master: [u8; 32],
    /// The account's devices: this machine's alone, with its certificate.
    devices: Vec<Device>,
}

/// Fails unless the run says it created the account's master key and holds its certificate.
fn assert_says_created(finished: &Finished) {
    let said = finished.shown().to_lowercase();
    assert!(
        said.contains("created") && said.contains("master key") && said.contains("certificate"),
        "{finished:?}"
    );
}

/// The username of the account of the session `token`, and whether it gives a GitHub login.
fn username_of(cloud: &Cloud, token: &str) -> (String, Option<Value>) {
    let (status, body) = cloud.session_of(token);
    assert_eq!(status, 200, "GET /session: {body}");
    let username = body["username"].as_str().expect("a username").to_owned();
    (username, body.get("github_login").cloned())
}

/// The certificate of the only device of `devices`; fails unless it passes the checks against
/// `master`.
fn only_cert(master: &[u8; 32], devices: &[Device]) -> Cert {
    let [device] = devices else {
        panic!("one device: {devices:?}");
    };
    let cert = device.cert.clone().expect("the device's certificate");
    skald_crypto::check_device(master, std::slice::from_ref(&cert), device.id)
        .expect("a certificate chained to the master key");
    cert
}

/// Signs a new machine on the host `host` in as the first device of `person`'s account, the person
/// signing in with Google on the verification page; fails unless it signs in naming `username`,
/// says it created the master key and holds a valid certificate of its host name.
fn first_with_google(
    cloud: &Cloud,
    browser: &Browser,
    person: &Person,
    username: &str,
    host: &str,
) -> First {
    let machine = Machine::new().with_host_name(host);
    let before = cloud.tokens().len();
    let finished =
        cloud.signed_in_with_google(&browser.page(), &machine, person, &[FIRST_DEVICE, YES]);
    assert!(finished.success, "{finished:?}");
    assert!(finished.shown().contains(username), "{finished:?}");
    assert_says_created(&finished);

    let tokens = cloud.tokens();
    assert_eq!(tokens.len(), before + 1, "one new session token");
    let token = tokens[before].clone();
    assert_eq!(username_of(cloud, &token), (username.to_owned(), None));
    let account = cloud.account_of(&token);
    let master = cloud.master(&token).expect("the account's master key");
    let devices = cloud.devices(&token);
    let cert = only_cert(&master, &devices);
    let body = cert.body();
    assert_eq!(body.account, account);
    assert_eq!(body.kind, DeviceKind::Client);
    assert_eq!(body.issuer, None, "issued by the master key");
    assert_eq!(body.name, host, "the host name");
    First {
        machine,
        finished,
        token,
        account,
        master,
        devices,
    }
}

/// Runs `skald sign-in` again on `first`'s machine: it finds its session, names `username` and asks
/// for no new code.
fn assert_finds_its_session(cloud: &Cloud, first: &First, username: &str) {
    let grants = cloud.grants().len();
    let again = first
        .machine
        .start(&[SIGN_IN, SERVER, &cloud.url, FIRST_DEVICE, YES])
        .finish();
    assert!(again.success, "{again:?}");
    assert!(again.shown().contains(username), "{again:?}");
    assert_eq!(cloud.grants().len(), grants, "a new code: {again:?}");
}

/// Runs `skald sign-out` on `machine`; fails unless it succeeds.
fn sign_out(machine: &Machine) {
    let out = machine.start(&[SIGN_OUT]).finish();
    assert!(out.success, "{out:?}");
}

/// Signs `first`'s machine, signed out, in again, the person approving as `person`, signed in with
/// Google; fails unless the run names `username` and the machine resumes its device on its
/// account: a new session, no master key sent, the same master key and the one device with its
/// certificate.
fn assert_resumes_with_google(
    cloud: &Cloud,
    browser: &Browser,
    first: &First,
    person: &Person,
    username: &str,
) {
    let mark = cloud.relay.mark();
    let again = cloud.signed_in_with_google(
        &browser.page(),
        &first.machine,
        person,
        &[FIRST_DEVICE, YES],
    );
    assert!(again.success, "{again:?}");
    assert!(again.shown().contains(username), "{again:?}");
    let token = &cloud.last_token();
    assert_ne!(token, &first.token);
    assert_eq!(cloud.account_of(token), first.account);
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");
    assert_eq!(cloud.master(token), Some(first.master));
    assert_eq!(
        cloud.devices(token),
        first.devices,
        "a new device or certificate"
    );
}

/// `skald` signs in while its person signs in with Google on the verification page: the account's
/// first machine creates the master key and holds a certificate chained to it, and run again it
/// finds its session.
#[test]
fn signs_in_while_its_person_signs_in_with_google() {
    let (cloud, browser) = setup(Providers::Both);
    let ada = cloud.person("110248495921238990001", "ada.cli@example.com");

    let first = first_with_google(&cloud, &browser, &ada, "ada-cli", "ada-workstation");

    assert_eq!(ids(&first.devices).len(), 1);
    assert_finds_its_session(&cloud, &first, "ada-cli");
}

/// As above, on a server with Google's settings only.
#[test]
fn signs_in_with_google_on_a_server_with_only_googles_settings() {
    let (cloud, browser) = setup(Providers::Google);
    let ada = cloud.person("110248495921238990011", "ada.only@example.com");

    let first = first_with_google(&cloud, &browser, &ada, "ada-only", "ada-laptop");

    assert!(!first.finished.shown().contains(&first.token), "the token");
    assert_finds_its_session(&cloud, &first, "ada-only");
}

/// Links Google as `person` to the account of the GitHub user `user`: signs `page`'s browser in
/// with GitHub, then clicks "Link Google" on the page of the account's sign-in methods.
fn link_google(cloud: &Cloud, page: &Page, user: &User, person: &Person) {
    let visit = page.goto(&cloud.github.sign_in_url(user.id));
    assert_eq!(
        visit.status,
        Some(200),
        "GitHub stand-in sign-in: {visit:?}"
    );
    page.goto_settled(&format!("{}/auth/github", cloud.public));
    let visit = page.goto(&cloud.provider().sign_in_url(&person.subject));
    assert_eq!(visit.status, Some(200), "OIDC stand-in sign-in: {visit:?}");
    let visit = page.goto(&format!("{}{METHODS_PAGE}", cloud.public));
    assert_eq!(visit.status, Some(200), "{visit:?}");
    let visit = page.click_settled(LINK_GOOGLE);
    assert!(visit.last().starts_with(&cloud.public), "{visit:?}");
}

/// The providers of the sign-in methods of the account of the session `token`, sorted.
fn providers_of(cloud: &Cloud, token: &str) -> Vec<String> {
    let (status, body) = cloud.get_as(METHODS, token);
    assert_eq!(status, 200, "GET {METHODS}: {body}");
    let methods = body["methods"].as_array().expect("methods");
    let mut providers: Vec<String> = methods
        .iter()
        .map(|method| method["provider"].as_str().expect("a provider").to_owned())
        .collect();
    providers.sort();
    providers
}

/// A machine signed in after a GitHub sign-in, signed out, resumes its device when its person
/// approves after a sign-in with the account's other method, Google: no new device, master key or
/// certificate.
#[test]
fn device_resumes_when_its_person_approves_with_the_accounts_other_method() {
    let (cloud, browser) = setup(Providers::Both);
    let bob = cloud.user(4_120_670_021, "bob-resumes-standin");
    let ada = cloud.person("110248495921238990021", "ada.resumes@example.com");
    let (machine, finished) = cloud.first_machine(&browser.page(), &bob, "bob-workstation");
    assert!(finished.success, "{finished:?}");
    assert_says_created(&finished);
    let token = cloud.last_token();
    let master = cloud.master(&token).expect("the account's master key");
    let devices = cloud.devices(&token);
    only_cert(&master, &devices);
    let first = First {
        machine,
        finished,
        account: cloud.account_of(&token),
        token,
        master,
        devices,
    };
    link_google(&cloud, &browser.page(), &bob, &ada);
    assert_eq!(providers_of(&cloud, &first.token), ["github", "google"]);
    sign_out(&first.machine);

    assert_resumes_with_google(&cloud, &browser, &first, &ada, "bob-resumes-standin");
}

/// A machine of one Google account, signed out and then approved by the Google user of another
/// account, is refused naming that account's username: the server refuses the session it got, the
/// machine keeps no token, and the device is neither registered on the other account nor changed.
/// Signed in again by its own person, it resumes.
#[test]
fn approval_by_the_google_user_of_another_account_is_refused_and_ends_that_session() {
    let (cloud, browser) = setup(Providers::Google);
    let ada = cloud.person("110248495921238990031", "ada.keeps@example.com");
    let carol = cloud.person("110248495921238990032", "carol.other@example.com");
    let first = first_with_google(&cloud, &browser, &ada, "ada-keeps", "ada-workstation");
    sign_out(&first.machine);
    let mark = cloud.relay.mark();
    let tokens = cloud.tokens().len();

    let refused = cloud.signed_in_with_google(
        &browser.page(),
        &first.machine,
        &carol,
        &[FIRST_DEVICE, YES],
    );

    assert!(!refused.success, "{refused:?}");
    assert!(refused.shown().contains("carol-other"), "{refused:?}");
    let given = cloud.tokens();
    assert_eq!(given.len(), tokens + 1, "the session the machine got");
    let carol_token = given.last().cloned().expect("a session token");
    let (status, body) = cloud.session_of(&carol_token);
    assert_eq!(status, 401, "the session it got still works: {body}");
    assert!(
        !refused.shown().contains(&carol_token),
        "the token was shown"
    );
    let kept: Vec<(PathBuf, String)> = cloud.token_files(&first.machine);
    assert_eq!(kept, Vec::new(), "a token kept");
    assert_eq!(cloud.count_sent(mark, REGISTER), 0, "registered on Carol's");
    assert_eq!(cloud.count_sent(mark, GIVE_MASTER), 0, "a master key sent");

    assert_resumes_with_google(&cloud, &browser, &first, &ada, "ada-keeps");
}
