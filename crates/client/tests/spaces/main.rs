//! Creating a team space in the CLI: a device with a valid certificate gets a space from the server
//! and stores its revision 1, which it signs as the space's first admin (`docs/protocol/membership.md`,
//! "Roster"). Against a `skald-server` with GitHub sign-in through the GitHub stand-in, on
//! PostgreSQL, behind the recording relay, which can lose the server's answers and rewrite them; a
//! test plays the person on the verification page in headless Chrome. Each machine keeps its state
//! in a directory of its own and its secrets in the 0600 file there.
//!
//! The CLI, as these tests use it:
//! - `skald create-space <name>`: creates a team space with the name on the server of the machine's
//!   session and stores its revision 1, signed by the device; a name the server or the client
//!   refuses fails the run, saying why; on a device without a certificate it fails, saying the
//!   device is `not linked` yet, and sends nothing; run again after a lost answer, it finishes the
//!   same space;
//! - `skald spaces`: lists the spaces the device accepted a revision of, one line per space, holding
//!   the space's name, the account's role in it (`admin` or `member`) and its handle.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../common/mod.rs"]
mod common;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;

#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;

use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use browser::Browser;
use cloud::{ASK_MASTER, Cloud};
use machine::{FIRST_DEVICE, Finished, Machine, NO, NOT_LINKED};
use relay::{Lose, strings_named};
use serde_json::{Value, json};
use skald_github_stand_in::User;
use skald_proto::{
    AccountId, Cbor, Cert, DeviceKind, Name, Policy, ROSTER_LABEL, Role, Roster, SpaceId,
};

/// The command that creates a space, and the one that lists the device's spaces.
const CREATE_SPACE: &str = "create-space";
const SPACES: &str = "spaces";

/// The request that creates a space, and the one that stores a space's revision, as their lines
/// start.
const CREATE: &str = "POST /spaces ";
const STORE: &str = "POST /spaces/";

/// The role the creator has in its space, as the space list shows it.
const ADMIN: &str = "admin";

/// An OIDC provider's issuer, for a sign-in method of a provider other than GitHub.
const GOOGLE: &str = "https://accounts.google.com";

/// An account and its first machine, signed in, which holds a valid certificate.
struct Account {
    user: User,
    machine: Machine,
    /// The machine's session token.
    token: String,
}

/// A server whose codes live for 2 minutes and poll every second, and a browser for the person.
fn setup() -> (Cloud, Browser) {
    (Cloud::start(120, 1), Browser::launch())
}

/// Signs the first machine of GitHub user `id` with `login` in, on the host `host`.
fn account(cloud: &Cloud, browser: &Browser, id: u64, login: &str, host: &str) -> Account {
    let user = cloud.user(id, login);
    let (machine, signed_in) = cloud.first_machine(&browser.page(), &user, host);
    assert!(signed_in.success, "{signed_in:?}");
    Account {
        user,
        machine,
        token: cloud.last_token(),
    }
}

/// Runs `skald create-space <name>` on `machine`.
fn create_space(machine: &Machine, name: &str) -> Finished {
    machine.start(&[CREATE_SPACE, name]).finish()
}

/// Runs `skald spaces` on `machine`; fails unless it succeeds.
fn spaces(machine: &Machine) -> Finished {
    let listed = machine.start(&[SPACES]).finish();
    assert!(listed.success, "{listed:?}");
    listed
}

/// The line of `listed` that names the space `name`, as a word of its own, if any.
fn line_of<'a>(listed: &'a Finished, name: &str) -> Option<&'a str> {
    let word = |part: &str| part == name;
    let lines: Vec<&str> = listed
        .stdout
        .lines()
        .filter(|line| {
            line.split(|char: char| !(char.is_ascii_alphanumeric() || char == '-'))
                .any(word)
        })
        .collect();
    assert!(lines.len() <= 1, "{name} is on several lines: {listed:?}");
    lines.first().copied()
}

/// Fails unless `machine`'s space list shows the space `name` with the role `admin` and `handle`.
fn assert_lists_as_admin(machine: &Machine, name: &str, handle: &str) {
    let listed = spaces(machine);
    let line = line_of(&listed, name).unwrap_or_else(|| panic!("{name} is not listed: {listed:?}"));
    let words: Vec<&str> = line
        .split(|char: char| !(char.is_ascii_alphanumeric() || char == '-'))
        .collect();
    assert!(words.contains(&ADMIN), "not as admin: {line:?}");
    assert!(
        words.contains(&handle),
        "not with the handle {handle}: {line:?}"
    );
}

/// The spaces the account of the session `token` gets, from the server: each one's ID and name.
fn server_spaces(cloud: &Cloud, token: &str) -> Vec<(SpaceId, String)> {
    let (status, body) = cloud.get_as("/spaces", token);
    assert_eq!(status, 200, "GET /spaces: {body}");
    body["spaces"]
        .as_array()
        .unwrap_or_else(|| panic!("GET /spaces: {body}"))
        .iter()
        .map(|space| {
            let id = space["space"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .unwrap_or_else(|| panic!("a space ID: {space}"));
            let name = space["name"]
                .as_str()
                .unwrap_or_else(|| panic!("a space name: {space}"));
            (id, name.to_owned())
        })
        .collect()
}

/// The revisions of `space` the server holds, for the session `token`: each one's bytes as stored,
/// and the revision they decode to.
fn server_revisions(cloud: &Cloud, token: &str, space: SpaceId) -> Vec<(Vec<u8>, Roster)> {
    cloud.revisions(token, space)
}

/// The handle of the account of the session `token`: its username, as the server gives it.
fn username_of(cloud: &Cloud, token: &str) -> String {
    let (status, session) = cloud.session_of(token);
    assert_eq!(status, 200, "GET /session: {session}");
    session["username"]
        .as_str()
        .unwrap_or_else(|| panic!("a username: {session}"))
        .to_owned()
}

/// Fails unless the server gives `account` exactly one space, `name`, with exactly one revision: a
/// revision 1 of the space, signed by the account's one device, which passes the checks of a new
/// space's revision against the account's master key, with the account the only member and admin,
/// with `handle` and that master key, no service agents or channels and a new space's policy. Gives
/// the space and the revision's bytes.
fn assert_one_first_revision(
    cloud: &Cloud,
    account: &Account,
    name: &str,
    handle: &str,
) -> (SpaceId, Vec<u8>) {
    let found = server_spaces(cloud, &account.token);
    assert_eq!(found.len(), 1, "the account's spaces: {found:?}");
    let (space, found_name) = found[0].clone();
    assert_eq!(found_name, name);
    let revisions = server_revisions(cloud, &account.token, space);
    assert_eq!(revisions.len(), 1, "the space's revisions: {revisions:?}");
    let (bytes, roster) = revisions[0].clone();
    assert_eq!(roster.to_cbor(), bytes, "the revision re-encodes otherwise");

    let id: AccountId = cloud.account_of(&account.token);
    let master = cloud
        .master(&account.token)
        .expect("the account's master key");
    let devices = cloud.devices(&account.token);
    assert_eq!(devices.len(), 1, "the account's devices: {devices:?}");
    let certs: Vec<Cert> = devices.iter().filter_map(|d| d.cert.clone()).collect();

    let body = roster.body();
    assert_eq!(body.space, space, "another space");
    assert_eq!(body.name.as_str(), name, "another name");
    assert_eq!(body.revision, 1);
    assert_eq!(body.prev, None, "revision 1 has a prev");
    assert_eq!(body.signer, devices[0].id, "signed by another device");
    let cert = skald_crypto::check_device(&master, &certs, body.signer)
        .unwrap_or_else(|err| panic!("the signer has no valid certificate: {err}"));
    assert_eq!(cert.body().account, id, "the signer is of another account");
    assert_eq!(cert.body().kind, DeviceKind::Client);
    skald_crypto::verify(
        &cert.body().signing,
        ROSTER_LABEL,
        &[roster.body_bytes()],
        roster.signature(),
    )
    .expect("the signature verifies with the signer's key");
    skald_crypto::check_roster(&roster, &certs).expect("revision 1 passes checks 3 and 4");

    assert_eq!(body.members.len(), 1, "members: {:?}", body.members);
    let member = &body.members[0];
    assert_eq!(member.account, id, "another account");
    assert_eq!(member.role, Role::Admin, "the creator is no admin");
    assert_eq!(member.handle.as_str(), handle, "another handle");
    assert_eq!(member.master, master, "another master key");
    assert_eq!(member.invite, None);
    assert_eq!(member.excluded, None);
    assert!(body.service_agents.is_empty(), "{:?}", body.service_agents);
    assert!(body.channels.is_empty(), "{:?}", body.channels);
    assert_eq!(body.policy, Policy::new_space());
    (space, bytes)
}

/// Runs `skald create-space <name>` on `account`'s machine; fails unless it succeeds naming the
/// space, the server holds the space's revision 1 as [`assert_one_first_revision`] says, with
/// `handle`, and the machine lists the space as its admin with `handle`. Gives the space and the
/// revision's bytes.
fn assert_creates(
    cloud: &Cloud,
    account: &Account,
    name: &str,
    handle: &str,
) -> (SpaceId, Vec<u8>) {
    let created = create_space(&account.machine, name);
    assert!(created.success, "{name}: {created:?}");
    assert!(created.shown().contains(name), "{created:?}");
    let first = assert_one_first_revision(cloud, account, name, handle);
    assert_lists_as_admin(&account.machine, name, handle);
    first
}

/// Whether `finished` says why the server or the client refused: one of the reasons the server
/// gave in its answers from `since` on, or one of `reasons`.
fn says_why(cloud: &Cloud, since: Instant, finished: &Finished, reasons: &[String]) -> bool {
    let shown = finished.shown();
    let lower = shown.to_lowercase();
    strings_named(&cloud.relay.answered_since(since), "detail")
        .iter()
        .any(|detail| shown.contains(detail.as_str()))
        || reasons
            .iter()
            .any(|reason| lower.contains(&reason.to_lowercase()))
}

/// The account's first device creates a space: it says so, and lists the space as its admin, with
/// its handle. The server holds revision 1 as the device signed it, and it passes the checks of a
/// new space's revision against the master key the device pinned, not one the server gives.
#[test]
fn first_device_creates_a_space_whose_revision_1_it_signed_as_its_admin() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_001,
        "ada-creates-standin",
        "ada-workstation",
    );
    let handle = username_of(&cloud, &ada.token);
    let before = spaces(&ada.machine);
    assert_eq!(line_of(&before, "acme"), None, "{before:?}");
    // A compromised server gives another master key: the revision keeps the pinned one.
    let other = URL_SAFE_NO_PAD.encode([0x5a; 32]);
    cloud.relay.rewrite(ASK_MASTER, move |body| {
        let mut answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        answer["master"] = json!(other);
        answer.to_string()
    });

    assert_creates(&cloud, &ada, "acme", &handle);
}

/// A device creates several spaces, each with a revision 1 of its own, and lists them all.
#[test]
fn spaces_lists_each_space_the_device_created() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_002,
        "ada-lists-standin",
        "ada-workstation",
    );
    let handle = username_of(&cloud, &ada.token);

    for name in ["acme", "globex"] {
        let created = create_space(&ada.machine, name);
        assert!(created.success, "{name}: {created:?}");
    }

    let found = server_spaces(&cloud, &ada.token);
    let mut names: Vec<&str> = found.iter().map(|(_, name)| name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["acme", "globex"]);
    for (space, name) in &found {
        let revisions = server_revisions(&cloud, &ada.token, *space);
        assert_eq!(revisions.len(), 1, "{name}: {revisions:?}");
        assert_eq!(revisions[0].1.body().revision, 1, "{name}");
        assert_lists_as_admin(&ada.machine, name, &handle);
    }
}

/// The relay loses the server's answer to `request` as `how` says: it closes the connection, or it
/// withholds the answer and the test kills the CLI. The same command run again, unless the run
/// finished by trying again itself, succeeds; it leaves one space with one revision 1, the space
/// created first and, if the server took it, the revision it took, byte for byte.
fn recovers_from_a_lost_answer(request: &str, how: Lose, user: (u64, &str)) {
    let (cloud, browser) = setup();
    let ada = account(&cloud, &browser, user.0, user.1, "ada-flaky-network");
    let handle = username_of(&cloud, &ada.token);
    let since = cloud.relay.mark();
    cloud.relay.lose_answer(request, how);

    let run = ada.machine.start(&[CREATE_SPACE, "acme"]);
    let done = match how {
        // It may try again itself and finish.
        Lose::Close => run.finish().success,
        Lose::Withhold => {
            cloud.kill_once_lost(run, request);
            false
        }
    };
    assert_eq!(cloud.relay.lost(), 1, "the server never answered {request}");
    let created = server_spaces(&cloud, &ada.token);
    assert_eq!(created.len(), 1, "the server created no space: {created:?}");
    let taken = server_revisions(&cloud, &ada.token, created[0].0);
    if request == STORE {
        assert_eq!(taken.len(), 1, "the server took no revision 1");
    }
    // A revision signed again now has another timestamp, and so other bytes.
    thread::sleep(Duration::from_millis(1100));

    if !done {
        let next = create_space(&ada.machine, "acme");
        assert!(next.success, "{next:?}");
    }

    assert!(
        cloud.count_sent(since, request) >= 2,
        "{request} was not sent again"
    );
    let (space, bytes) = assert_one_first_revision(&cloud, &ada, "acme", &handle);
    assert_eq!(space, created[0].0, "another space");
    if let Some((first, _)) = taken.first() {
        assert_eq!(
            &bytes, first,
            "another revision 1 than the one the server took"
        );
    }
    assert_lists_as_admin(&ada.machine, "acme", &handle);
}

#[test]
fn creation_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(CREATE, Lose::Close, (4_120_671_101, "ada-lost-creation"));
}

#[test]
fn creation_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        CREATE,
        Lose::Withhold,
        (4_120_671_102, "ada-killed-creation"),
    );
}

#[test]
fn revision_1_answer_dropped_by_the_proxy_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(STORE, Lose::Close, (4_120_671_103, "ada-lost-revision"));
}

#[test]
fn revision_1_answer_lost_with_the_cli_killed_is_recovered_by_the_next_run() {
    recovers_from_a_lost_answer(
        STORE,
        Lose::Withhold,
        (4_120_671_104, "ada-killed-revision"),
    );
}

/// A name another account's space has creates nothing: the run fails saying why, the account gets
/// no space and lists none, and the other space keeps its one revision.
#[test]
fn taken_name_creates_nothing_and_says_why() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_201,
        "ada-taken-standin",
        "ada-workstation",
    );
    let bob = account(
        &cloud,
        &browser,
        4_120_671_202,
        "bob-owner-standin",
        "bob-workstation",
    );
    let bob_handle = username_of(&cloud, &bob.token);
    let (_, bob_revision) = assert_creates(&cloud, &bob, "globex", &bob_handle);
    let since = cloud.relay.mark();

    let refused = create_space(&ada.machine, "globex");

    assert!(!refused.success, "{refused:?}");
    assert!(
        says_why(&cloud, since, &refused, &["taken".to_owned()]),
        "it does not say why: {refused:?}"
    );
    assert_eq!(cloud.count_sent(since, STORE), 0, "a revision was sent");
    assert_eq!(server_spaces(&cloud, &ada.token), Vec::new());
    assert_eq!(line_of(&spaces(&ada.machine), "globex"), None);
    let (_, still) = assert_one_first_revision(&cloud, &bob, "globex", &bob_handle);
    assert_eq!(still, bob_revision, "the other space's revision changed");
}

/// A name that is no valid `name` of an agent address creates nothing: the run fails saying why,
/// and the account gets no space and lists none.
#[test]
fn invalid_name_creates_nothing_and_says_why() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_301,
        "ada-invalid-standin",
        "ada-workstation",
    );
    let too_long = "a".repeat(64);
    let invalid = ["acme_team", "acme.team", "acme-", "äcme", too_long.as_str()];

    for name in invalid {
        let reason = name
            .parse::<Name>()
            .expect_err("an invalid name")
            .to_string();
        let since = cloud.relay.mark();

        let refused = create_space(&ada.machine, name);

        assert!(!refused.success, "{name:?} was taken: {refused:?}");
        assert!(
            says_why(&cloud, since, &refused, &[reason]),
            "{name:?}: it does not say why: {refused:?}"
        );
        assert_eq!(
            cloud.count_sent(since, STORE),
            0,
            "{name:?}: a revision sent"
        );
        assert_eq!(server_spaces(&cloud, &ada.token), Vec::new(), "{name:?}");
    }
    let listed = spaces(&ada.machine);
    for name in invalid {
        assert!(!listed.stdout.contains(name), "{name:?}: {listed:?}");
    }
}

/// A device of the account without a certificate creates nothing: the run fails saying the device
/// is not linked yet and sends no request to create a space; the account gets no space.
#[test]
fn device_without_a_certificate_creates_nothing_and_says_it_is_not_linked() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_401,
        "ada-unlinked-standin",
        "ada-workstation",
    );
    let laptop = Machine::new().with_host_name("ada-unlinked-laptop");
    let page = browser.page();
    let waiting = cloud.signed_in(&page, &laptop, &ada.user, &[FIRST_DEVICE, NO]);
    assert!(waiting.success, "the second machine: {waiting:?}");
    let token = cloud.last_token();
    assert_ne!(
        token, ada.token,
        "the second machine has a session of its own"
    );
    let since = cloud.relay.mark();

    let refused = create_space(&laptop, "acme");

    assert!(!refused.success, "{refused:?}");
    assert!(
        refused.shown().to_lowercase().contains(NOT_LINKED),
        "it does not say it is not linked: {refused:?}"
    );
    assert_eq!(cloud.count_sent(since, CREATE), 0, "a space was requested");
    assert_eq!(cloud.count_sent(since, STORE), 0, "a revision was sent");
    assert_eq!(server_spaces(&cloud, &token), Vec::new());
    assert_eq!(server_spaces(&cloud, &ada.token), Vec::new());
}

/// A GitHub login that is no valid `name` gives a valid handle: the account's username, which the
/// server made of the login.
#[test]
fn login_that_is_no_valid_name_gives_the_username_as_a_valid_handle() {
    let (cloud, browser) = setup();
    let login = "Ada_Lovelace_Standin";
    assert!(login.parse::<Name>().is_err(), "the login is a valid name");
    let ada = account(&cloud, &browser, 4_120_671_501, login, "ada-workstation");
    let handle = username_of(&cloud, &ada.token);
    assert_eq!(handle, "ada-lovelace-standin");

    assert_creates(&cloud, &ada, "acme", &handle);
}

/// The handle is the account's username as the server gives it when the space is created: one set
/// with `skald set-username`, not one made of the GitHub login.
#[test]
fn handle_is_the_username_set_with_the_cli() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_601,
        "ada-renamed-standin",
        "ada-workstation",
    );
    let set = ada.machine.start(&["set-username", "ada-of-acme"]).finish();
    assert!(set.success, "{set:?}");
    assert_eq!(username_of(&cloud, &ada.token), "ada-of-acme");

    assert_creates(&cloud, &ada, "acme", "ada-of-acme");
}

/// An account whose GitHub sign-in method an operator replaced with one of another provider has no
/// GitHub login: its handle is its username.
#[test]
fn account_without_a_github_sign_in_method_gets_its_username_as_handle() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_701,
        "Ada_Google_Standin",
        "ada-workstation",
    );
    let handle = username_of(&cloud, &ada.token);
    let id = cloud.account_of(&ada.token);
    cloud.execute(&format!(
        "UPDATE sign_in_methods SET provider = '{GOOGLE}', subject = '108204371559281736499',
                login = NULL
         WHERE account_id = '{id}'"
    ));
    let (_, session) = cloud.session_of(&ada.token);
    assert_eq!(session.get("github_login"), None, "{session}");
    assert_eq!(session["username"], json!(handle), "{session}");

    assert_creates(&cloud, &ada, "acme", &handle);
    assert_ne!(handle, ada.user.login, "the handle is the GitHub login");
}

/// A revision 1 the server refuses drops its space: the run fails saying why, and the device no
/// longer lists the space. The account has the space on the server without a revision already, so
/// the server answers the request that creates it with 200, which the relay rewrites to give the
/// device a space the server does not have; the server refuses its revision 1.
#[test]
fn refused_revision_1_drops_its_space_and_says_why() {
    let (cloud, browser) = setup();
    let ada = account(
        &cloud,
        &browser,
        4_120_671_801,
        "ada-refused-standin",
        "ada-workstation",
    );
    let id = cloud.account_of(&ada.token);
    cloud.execute(&format!(
        "INSERT INTO spaces (id, name, creator_id) VALUES ('{}', 'acme', '{id}')",
        SpaceId::generate()
    ));
    let unknown = SpaceId::generate().to_string();
    cloud.relay.rewrite(CREATE, move |body| {
        let mut answer: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        answer["space"] = json!(unknown);
        answer.to_string()
    });
    let since = cloud.relay.mark();

    let refused = create_space(&ada.machine, "acme");

    assert!(!refused.success, "{refused:?}");
    assert_eq!(cloud.count_sent(since, STORE), 1, "no revision 1 was sent");
    assert!(
        says_why(&cloud, since, &refused, &[]),
        "it does not say why: {refused:?}"
    );
    assert_eq!(line_of(&spaces(&ada.machine), "acme"), None);
}
