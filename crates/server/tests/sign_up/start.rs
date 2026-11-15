//! The sign-up setting at start: `open` by default, with a warning in the start log while it is
//! open; none while it is closed; an invalid value stops the server, naming the setting. The
//! setting and its lists work from the environment too.

use std::env;
use std::fs::{self, File};
use std::process::{self, Command};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::common::browser::Browser;
use super::common::sign_in::{CLIENT_ID as GITHUB_CLIENT_ID, SignIn, github_stand_in};
use super::common::{SERVER, TestDb};
use super::*;

/// The lines of `log` that warn about open sign-up: with `warning`, naming sign-up.
fn warnings(log: &str) -> Vec<String> {
    log.lines()
        .map(str::to_lowercase)
        .filter(|line| line.contains("warning"))
        .filter(|line| line.contains("sign-up") || line.contains("sign up"))
        .collect()
}

/// Runs a server with `args` and the environment variables `envs`, its output in a file, until it
/// exits or a second after it listens, for up to 30 s, then kills it. Gives whether it exited
/// failing, and everything it printed, stdout and stderr, in any order of its lines.
fn start_output(args: &[&str], envs: &[(&str, &str)]) -> (bool, String) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = env::temp_dir().join(format!(
        "skald-sign-up-start-{}-{}.log",
        process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let log = File::create(&path).expect("create the server's log");
    let mut child = Command::new(SERVER)
        .args(args)
        .envs(envs.iter().copied())
        .stderr(log.try_clone().expect("open the server's log twice"))
        .stdout(log)
        .spawn()
        .expect("start skald-server");
    let deadline = Instant::now() + Duration::from_secs(30);
    let failed = loop {
        if let Some(status) = child.try_wait().expect("wait for skald-server") {
            break !status.success();
        }
        let printed = fs::read_to_string(&path).unwrap_or_default();
        if printed.contains("skald-server: listening on ") {
            thread::sleep(Duration::from_secs(1));
            break false;
        }
        if Instant::now() > deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let _ = child.kill();
    let _ = child.wait();
    let printed = fs::read_to_string(&path).expect("read the server's log");
    let _ = fs::remove_file(&path);
    (failed, printed)
}

/// What a server with GitHub's settings and the extra arguments `args` and environment variables
/// `envs` prints from its start until a second after it listens; fails unless it listens.
fn start_log(args: &[&str], envs: &[(&str, &str)]) -> String {
    let db = TestDb::create();
    let github = github_stand_in("http://127.0.0.1:9");
    let url = db.url();
    let mut all = vec![
        "--listen",
        "127.0.0.1:0",
        "--database-url",
        &url,
        "--public-url",
        "http://127.0.0.1:9",
        "--github-client-id",
        GITHUB_CLIENT_ID,
        "--github-url",
        github.url(),
        "--github-api-url",
        github.url(),
    ];
    all.extend_from_slice(args);
    let mut all_envs = vec![(GITHUB_SECRET_ENV, GITHUB_CLIENT_SECRET)];
    all_envs.extend_from_slice(envs);
    let (_, printed) = start_output(&all, &all_envs);
    assert!(
        printed.contains("skald-server: listening on "),
        "skald-server did not start: {printed}"
    );
    printed
}

/// Without the setting, sign-up is open: an unlisted GitHub login and a Google account sign up,
/// and the start log warns that any GitHub or Google account can sign up.
#[test]
fn open_is_the_default_and_warns_at_start() {
    let log = start_log(&[], &[]);
    assert_eq!(
        warnings(&log).len(),
        1,
        "one warning of open sign-up: {log}"
    );

    let browser = Browser::launch();
    let google = default_server();
    let bob = github_user(&google, 7_310_000_301, "bob-open");
    github_signed_up(&google, &browser.page(), &bob);
    let dan = person(
        &google,
        "110248495921238991301",
        "dan.open@example.org",
        false,
        None,
    );
    google.signed_up(&browser, &dan);
}

/// `--sign-up open` and `SKALD_SIGN_UP=open` warn as the default does; `closed` warns of nothing.
#[test]
fn only_open_sign_up_warns_at_start() {
    for (args, envs) in [
        (vec![SIGN_UP, "open"], vec![]),
        (vec![], vec![(SIGN_UP_ENV, "open")]),
    ] {
        let log = start_log(&args, &envs);
        assert_eq!(warnings(&log).len(), 1, "{args:?} {envs:?}: {log}");
    }
    for (args, envs) in [
        (vec![SIGN_UP, "closed", GITHUB_LOGINS, "ada-start"], vec![]),
        (vec![SIGN_UP, "closed"], vec![]),
        (vec![], vec![(SIGN_UP_ENV, "closed")]),
    ] {
        let log = start_log(&args, &envs);
        assert_eq!(
            warnings(&log),
            Vec::<String>::new(),
            "{args:?} {envs:?}: {log}"
        );
    }
}

/// An invalid sign-up setting, given as an argument or in the environment, stops the server at
/// start, naming the setting.
#[test]
fn an_invalid_setting_stops_the_start_naming_it() {
    let db = TestDb::create();
    let url = db.url();
    let base = ["--listen", "127.0.0.1:0", "--database-url", url.as_str()];
    for value in ["ajar", "", "true", "closed,open"] {
        let mut args = base.to_vec();
        args.extend([SIGN_UP, value]);
        let (refused, printed) = start_output(&args, &[]);
        assert!(refused, "--sign-up {value:?} starts: {printed}");
        assert!(
            printed.contains(SIGN_UP) || printed.contains(SIGN_UP_ENV),
            "--sign-up {value:?}: {printed}"
        );
    }
    let (refused, printed) = start_output(&base, &[(SIGN_UP_ENV, "ajar")]);
    assert!(refused, "{SIGN_UP_ENV}=ajar starts: {printed}");
    assert!(
        printed.contains(SIGN_UP) || printed.contains(SIGN_UP_ENV),
        "{SIGN_UP_ENV}=ajar: {printed}"
    );
}

/// The setting and the GitHub list from the environment, as `.env` gives them: the listed login
/// signs up, another is refused.
#[test]
fn closed_sign_up_and_its_github_list_from_the_environment() {
    let browser = Browser::launch();
    let stand = SignIn::on_with(
        TestDb::create(),
        &[],
        &[
            (SIGN_UP_ENV, "closed"),
            (GITHUB_LOGINS_ENV, "ada-env,cy-env"),
        ],
    );
    let ada = stand.user(7_310_000_311, "ada-env");
    let page = browser.page();
    let visit = stand.sign_in(&page, &ada);
    assert_eq!(visit.status, Some(200), "{visit:?}: {}", page.text());
    assert_eq!(stand.account(&page).github_login, "ada-env");

    let bob = stand.user(7_310_000_312, "bob-env");
    let page = browser.page();
    let visit = stand.sign_in(&page, &bob);
    assert_eq!(visit.status, Some(403), "{visit:?}: {}", page.text());
    assert!(html(&page).contains(REFUSED), "{}", page.text());
    stand.assert_signed_out(&page);
    assert_eq!(stand.rows_of(&bob), Vec::<String>::new());
}

/// The Google lists from the environment: the listed email and the account of the listed domain
/// sign up, another is refused.
#[test]
fn google_lists_from_the_environment() {
    let browser = Browser::launch();
    let google = restarted(
        default_server(),
        &[],
        &[
            (SIGN_UP_ENV, "closed"),
            (GOOGLE_EMAILS_ENV, "grace.env@example.com"),
            (GOOGLE_DOMAINS_ENV, "acme-env.example"),
        ],
    );
    let grace = person(
        &google,
        "110248495921238991311",
        "grace.env@example.com",
        true,
        None,
    );
    google.signed_up(&browser, &grace);
    let cy = person(
        &google,
        "110248495921238991312",
        "cy@acme-env.example",
        true,
        Some("acme-env.example"),
    );
    google.signed_up(&browser, &cy);
    let dan = person(
        &google,
        "110248495921238991313",
        "dan.env@example.org",
        true,
        None,
    );
    let page = browser.page();
    let visit = google.sign_in(&page, &dan);
    assert_refused(&google, &page, &visit, "an unlisted email");
    google.assert_no_account(&dan, "an unlisted email");
}
