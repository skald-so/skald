//! `skald sign-in` on a machine without a browser: the device flow (RFC 8628) against a
//! `skald-server` with GitHub sign-in through the GitHub stand-in, on PostgreSQL, with a test
//! playing the person on the verification page in headless Chrome. A recording relay before the
//! server shows what the machine sent and got. Each machine keeps its state in a directory of its
//! own and its secrets in the 0600 file there, never in the system's key storage.

#[allow(dead_code)]
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[allow(dead_code)]
mod cloud;
#[allow(dead_code)]
#[path = "../common/mod.rs"]
mod common;
#[allow(dead_code)]
mod machine;
#[allow(dead_code)]
mod relay;

use std::thread;
use std::time::{Duration, Instant};

use browser::{Browser, Page};
use cloud::{APPROVE, Cloud, DENY, WAIT, shown_grant};
use machine::{Finished, Machine};
use skald_github_stand_in::User;

/// The polls of the token endpoint the machine sent so far, by when they came.
fn polls(cloud: &Cloud) -> Vec<Instant> {
    let mut sent = cloud.relay.sent();
    sent.retain(|request| request.head.starts_with("post /auth/token "));
    sent.into_iter().map(|request| request.at).collect()
}

/// Waits until the machine has polled `count` times in all.
fn wait_for_polls(cloud: &Cloud, count: usize) {
    let deadline = Instant::now() + WAIT;
    while polls(cloud).len() < count {
        assert!(Instant::now() < deadline, "fewer than {count} polls");
        thread::sleep(Duration::from_millis(50));
    }
}

/// Signs `machine` in as `user`, the person approving at `page`; gives the run's output and the
/// session token the server gave.
fn sign_in(cloud: &Cloud, page: &Page, machine: &Machine, user: &User) -> (Finished, String) {
    let before = cloud.grants().len();
    let mut run = machine.sign_in(&cloud.url);
    let grant = shown_grant(cloud, &mut run, before);
    cloud.decide(page, user, &grant, APPROVE);
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");
    let token = cloud.last_token();
    (finished, token)
}

/// Checks that a run of `machine` after a sign-in that stored nothing is not signed in: it asks for
/// a new code and sends no session token.
fn assert_not_signed_in(cloud: &Cloud, machine: &Machine) {
    let before = cloud.grants().len();
    let mark = cloud.relay.mark();
    let mut run = machine.sign_in(&cloud.url);
    let grant = shown_grant(cloud, &mut run, before);
    assert_ne!(grant.user_code, cloud.grants()[before - 1].user_code);
    let sent = cloud.relay.sent_since(mark);
    assert!(
        sent.iter()
            .all(|request| !request.head.contains("\nauthorization:")),
        "a session token was sent: {sent:?}"
    );
}

#[test]
fn shows_the_address_and_the_code_and_signs_in_once_the_person_approves() {
    let interval = 2;
    let cloud = Cloud::start(120, interval);
    let browser = Browser::launch();
    let page = browser.page();
    let ada = cloud.user(4_120_665_001, "ada-cli-standin");
    let machine = Machine::new();

    let mut run = machine.sign_in(&cloud.url);
    // It shows the address and the code, and waits for the person while it polls.
    let grant = shown_grant(&cloud, &mut run, 0);
    wait_for_polls(&cloud, 2);
    assert_eq!(cloud.tokens(), Vec::<String>::new());
    cloud.decide(&page, &ada, &grant, APPROVE);
    let finished = run.finish();

    assert!(finished.success, "{finished:?}");
    assert!(finished.shown().contains(&ada.login), "{finished:?}");
    let tokens = cloud.tokens();
    assert_eq!(tokens.len(), 1, "one session token");
    assert_eq!(cloud.login_of(&tokens[0]), ada.login);
    // It polls as the server says: never sooner than the interval, so never told to slow down.
    let polls = polls(&cloud);
    for pair in polls.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap >= Duration::from_secs(interval) - Duration::from_millis(100),
            "polls {gap:?} apart, the interval is {interval} s"
        );
    }
    assert!(!cloud.relay.answered().contains("slow_down"));
}

#[test]
fn run_again_finds_the_session_and_says_which_account() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let page = browser.page();
    let ada = cloud.user(4_120_665_002, "ada-again-standin");
    let machine = Machine::new();
    let (_, token) = sign_in(&cloud, &page, &machine, &ada);

    let mark = cloud.relay.mark();
    let again = machine.sign_in(&cloud.url).finish();

    assert!(again.success, "{again:?}");
    assert!(again.shown().contains(&ada.login), "{again:?}");
    // It checks the session with the server, and asks for no new code.
    let bearer = format!("\nauthorization: bearer {}", token.to_lowercase());
    let sent = cloud.relay.sent_since(mark);
    assert!(
        sent.iter().any(|request| request.head.contains(&bearer)),
        "the session token was not sent: {sent:?}"
    );
    assert_eq!(cloud.grants().len(), 1, "a new code: {again:?}");
    assert_eq!(cloud.tokens().len(), 1);
}

#[test]
fn denied_code_signs_nothing_in_and_stores_no_token() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let page = browser.page();
    let ada = cloud.user(4_120_665_003, "ada-denies-standin");
    let machine = Machine::new();

    let mut run = machine.sign_in(&cloud.url);
    let grant = shown_grant(&cloud, &mut run, 0);
    cloud.decide(&page, &ada, &grant, DENY);
    let finished = run.finish();

    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains("denied"),
        "a clear message: {finished:?}"
    );
    assert_eq!(cloud.tokens(), Vec::<String>::new());
    assert_not_signed_in(&cloud, &machine);
}

#[test]
fn expired_code_signs_nothing_in_and_stores_no_token() {
    let cloud = Cloud::start(3, 1);
    let machine = Machine::new();

    let mut run = machine.sign_in(&cloud.url);
    shown_grant(&cloud, &mut run, 0);
    let finished = run.finish();

    assert!(!finished.success, "{finished:?}");
    assert!(
        finished.shown().to_lowercase().contains("expire"),
        "a clear message: {finished:?}"
    );
    assert_eq!(cloud.tokens(), Vec::<String>::new());
    assert_not_signed_in(&cloud, &machine);
}

/// The session token, as the server gave it, is in one file only: the secrets file in the state
/// directory, with 0600 permissions. The local state, SQLite, holds the server's address and no
/// secret; nothing the machine printed shows the token.
#[cfg(unix)]
#[test]
fn session_is_kept_only_in_the_secrets_file_with_0600_permissions() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let page = browser.page();
    let ada = cloud.user(4_120_665_004, "ada-secrets-standin");
    let machine = Machine::new();
    let (finished, token) = sign_in(&cloud, &page, &machine, &ada);
    let again = machine.sign_in(&cloud.url).finish();
    assert!(again.success, "{again:?}");

    let holders = machine.files_holding(&token);
    assert_eq!(holders.len(), 1, "files holding the token: {holders:?}");
    let secrets = &holders[0];
    assert!(secrets.starts_with(machine.state_dir()), "{secrets:?}");
    let mode = fs::metadata(secrets)
        .expect("the secrets file")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "{secrets:?}: {mode:o}");

    let state = machine.sqlite_files();
    assert!(!state.is_empty(), "no SQLite state in {:?}", machine.root());
    assert!(
        !state.contains(secrets),
        "the token in the state: {secrets:?}"
    );
    let address = cloud.url.trim_start_matches("http://");
    let holding_address = machine.files_holding(address);
    assert!(
        state.iter().any(|file| holding_address.contains(file)),
        "the state holds no server address: {state:?}"
    );

    for output in [&finished, &again] {
        assert!(!output.shown().contains(&token), "the token was shown");
    }
}

#[test]
fn machines_with_their_own_state_directories_keep_their_own_sessions() {
    let cloud = Cloud::start(120, 1);
    let browser = Browser::launch();
    let ada = cloud.user(4_120_665_005, "ada-first-machine-standin");
    let bob = cloud.user(4_120_665_006, "bob-second-machine-standin");
    let first = Machine::new();
    let second = Machine::new();
    sign_in(&cloud, &browser.page(), &first, &ada);
    sign_in(&cloud, &browser.page(), &second, &bob);

    for (machine, user, other) in [(&first, &ada, &bob), (&second, &bob, &ada)] {
        let again = machine.sign_in(&cloud.url).finish();
        assert!(again.success, "{again:?}");
        assert!(again.shown().contains(&user.login), "{again:?}");
        assert!(!again.shown().contains(&other.login), "{again:?}");
    }
    assert_eq!(cloud.grants().len(), 2);
}
