//! Acceptance tests of `skald sign-in` against a server whose sign-up is closed: when the person's
//! sign-up is refused on the verification page, `skald sign-in` ends at once, saying sign-up is
//! closed on the server, instead of waiting for its code to expire; a listed person's machine signs
//! in. The server is the `skald-server` binary of this build, which `cargo build --workspace` puts
//! next to this test binary's directory, with GitHub sign-in through the GitHub stand-in, on
//! PostgreSQL; the person is a test in headless Chrome.
//!
//! The server's settings these tests use: `--sign-up closed` and `--sign-up-github-logins`, a
//! comma-separated list of the GitHub logins that may sign up.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

#[path = "../sign_in/machine.rs"]
mod machine;

#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../common/mod.rs"]
mod common;

use std::env;
use std::fs::{self, File};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{self, Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use browser::{APPROVE, Browser, CODE_FIELD, Page, SIGN_IN_GITHUB, SUBMIT};
use machine::{FIRST_DEVICE, Machine, Run, SERVER, SIGN_IN, YES};
use skald_github_stand_in::{App, GitHub, User};
use sqlx::AssertSqlSafe;
use tokio::runtime::Runtime;

/// The fake OAuth App's client ID and secret.
const CLIENT_ID: &str = "Ov23li0signup0closed";
const CLIENT_SECRET: &str = "standin-secret-sign-up-closed-0f1e2d3c4b5a6978";
/// GitHub's callback under the server's public address.
const CALLBACK: &str = "/auth/github/callback";
/// The only GitHub login the server lets sign up.
const LISTED: &str = "ada-cli-listed";
/// How long the device codes work: far longer than any wait of these tests.
const CODE_LIFETIME: &str = "600";

/// The `skald-server` binary of this build: in the profile's directory, above this test binary's,
/// which is `deps/` in it or, in Cargo's newer build directory layout, deeper.
fn server_binary() -> PathBuf {
    let exe = env::current_exe().expect("this test binary");
    let name = format!("skald-server{}", env::consts::EXE_SUFFIX);
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join(&name))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!(
                "no {name} above {}: build it first, cargo build --workspace, as script/test does",
                exe.display()
            )
        })
}

/// A new database on the PostgreSQL server of the tests; dropped on drop.
struct Db {
    name: String,
}

impl Db {
    fn create() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let db = Db {
            name: format!(
                "skald_client_test_{}_{}",
                process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ),
        };
        db.admin(&[
            format!("DROP DATABASE IF EXISTS {}", db.name),
            format!("CREATE DATABASE {}", db.name),
        ]);
        db
    }

    fn url(&self) -> String {
        format!("{}/{}", common::server(), self.name)
    }

    fn admin(&self, statements: &[String]) {
        let runtime = Runtime::new().expect("start a tokio runtime");
        runtime.block_on(async {
            let mut conn = common::connect(&format!("{}/postgres", common::server()))
                .await
                .expect("connect to PostgreSQL");
            for sql in statements {
                sqlx::raw_sql(AssertSqlSafe(sql.as_str()))
                    .execute(&mut conn)
                    .await
                    .unwrap_or_else(|err| panic!("{sql}: {err}"));
            }
            common::close(conn).await;
        });
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let statement = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", self.name);
        self.admin(&[statement]);
    }
}

/// A `skald-server` process with GitHub sign-in through the stand-in and sign-up closed but for
/// [`LISTED`]; killed on drop.
struct Closed {
    /// Where machines reach it, `http://127.0.0.1:<port>`.
    url: String,
    github: GitHub,
    child: Child,
    /// The file its output goes to; removed on drop.
    log: PathBuf,
    db: Db,
}

impl Closed {
    fn start() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let db = Db::create();
        let log = env::temp_dir().join(format!(
            "skald-client-sign-up-{}-{}.log",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let port = TcpListener::bind("127.0.0.1:0")
                .and_then(|listener| listener.local_addr())
                .expect("a free local port")
                .port();
            // Chrome resolves every *.test host to 127.0.0.1.
            let public = format!("http://skald.test:{port}");
            let github = GitHub::start(App {
                client_id: CLIENT_ID.to_owned(),
                client_secret: CLIENT_SECRET.to_owned(),
                callback: Some(format!("{public}{CALLBACK}")),
            });
            let listen = format!("127.0.0.1:{port}");
            let output = File::create(&log).expect("create the server's log");
            let mut child = Command::new(server_binary())
                .env("SKALD_GITHUB_CLIENT_SECRET", CLIENT_SECRET)
                .stdin(Stdio::null())
                .stderr(output.try_clone().expect("open the server's log twice"))
                .stdout(output)
                .args(["--listen", &listen, "--database-url", &db.url()])
                .args(["--public-url", &public, "--github-client-id", CLIENT_ID])
                .args([
                    "--github-url",
                    github.url(),
                    "--github-api-url",
                    github.url(),
                ])
                .args(["--device-code-lifetime", CODE_LIFETIME])
                .args(["--device-poll-interval", "1"])
                .args(["--sign-up", "closed", "--sign-up-github-logins", LISTED])
                .spawn()
                .expect("start skald-server");
            let printed = loop {
                let printed = fs::read_to_string(&log).unwrap_or_default();
                if printed.contains("skald-server: listening on ") {
                    break None;
                }
                if child.try_wait().expect("wait for skald-server").is_some() {
                    break Some(fs::read_to_string(&log).unwrap_or_default());
                }
                assert!(
                    Instant::now() < deadline,
                    "skald-server did not listen: {printed}"
                );
                thread::sleep(Duration::from_millis(50));
            };
            match printed {
                None => {
                    return Closed {
                        url: format!("http://{listen}"),
                        github,
                        child,
                        log,
                        db,
                    };
                }
                Some(printed)
                    if printed.to_lowercase().contains("address already in use")
                        && Instant::now() < deadline => {}
                Some(printed) => panic!("skald-server did not start: {printed}"),
            }
        }
    }

    /// Adds a GitHub user to the stand-in and gives it.
    fn user(&self, id: u64, login: &str) -> User {
        let user = User::new(id, login);
        self.github.put_user(user.clone());
        user
    }

    /// What the server printed so far.
    fn printed(&self) -> String {
        fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for Closed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_file(&self.log);
    }
}

/// Starts `skald sign-in` on `machine` as the account's first device; gives the run and the
/// verification page with its code filled in, once the run shows it.
fn start_sign_in(server: &Closed, machine: &Machine) -> (Run, String) {
    let mut run = machine.start(&[SIGN_IN, SERVER, &server.url, FIRST_DEVICE, YES]);
    run.wait_to_show(&["user_code="]);
    let shown = run.shown();
    let complete = shown
        .split_whitespace()
        .find(|word| word.contains("/device?user_code="))
        .map(|word| word.trim_end_matches([',', '.', ';']).to_owned())
        .unwrap_or_else(|| panic!("no verification page with its code: {shown}"));
    (run, complete)
}

/// The person at `page`, signed in to GitHub as `user`, opens the verification page `complete`
/// and signs in there with GitHub; gives the status of where it ended.
fn sign_in_on_the_page(server: &Closed, page: &Page, user: &User, complete: &str) -> Option<u16> {
    let visit = page.goto(&server.github.sign_in_url(user.id));
    assert_eq!(visit.status, Some(200), "stand-in sign-in: {visit:?}");
    let opened = page.goto_settled(complete);
    assert_eq!(opened.status, Some(200), "{opened:?}");
    assert_eq!(page.count(SIGN_IN_GITHUB), 1, "{}", page.text());
    page.click_settled(&format!("{SIGN_IN_GITHUB} >> nth=0"))
        .status
}

/// Whether `text` says that sign-up is closed.
fn says_closed(text: &str) -> bool {
    let text = text.to_lowercase();
    (text.contains("sign-up") || text.contains("sign up")) && text.contains("closed")
}

/// A person whose GitHub login is not listed signs in on the verification page: `skald sign-in`
/// ends within seconds, though its code works for ten minutes, fails and says sign-up is closed on
/// the server.
#[test]
fn sign_in_ends_at_once_when_sign_up_is_refused() {
    let server = Closed::start();
    let browser = Browser::launch();
    let bob = server.user(7_320_000_001, "bob-cli-unlisted");
    let machine = Machine::new().with_host_name("bob-laptop");

    let (run, complete) = start_sign_in(&server, &machine);
    let page = browser.page();
    let status = sign_in_on_the_page(&server, &page, &bob, &complete);
    assert_eq!(status, Some(403), "refused on the page: {}", page.text());

    let refused = Instant::now();
    let finished = run.finish();
    let took = refused.elapsed();
    assert!(
        took < Duration::from_secs(20),
        "skald sign-in ran {took:?} after the refusal: {finished:?}"
    );
    assert!(!finished.success, "{finished:?}");
    assert!(says_closed(&finished.shown()), "{finished:?}");
    assert!(
        !finished.shown().to_lowercase().contains("expired"),
        "{finished:?}"
    );
    let printed = server.printed();
    assert!(!printed.contains(CLIENT_SECRET), "{printed}");
}

/// The control: the listed login signs in on the verification page and approves the code, and the
/// machine signs in to the new account.
#[test]
fn listed_login_signs_in_by_the_cli() {
    let server = Closed::start();
    let browser = Browser::launch();
    let ada = server.user(7_320_000_002, LISTED);
    let machine = Machine::new().with_host_name("ada-laptop");

    let (run, complete) = start_sign_in(&server, &machine);
    let page = browser.page();
    let status = sign_in_on_the_page(&server, &page, &ada, &complete);
    assert_eq!(status, Some(200), "{}", page.text());
    if page.count(CODE_FIELD) == 1 && page.count(APPROVE) == 0 {
        page.click(SUBMIT);
    }
    let visit = page.click(APPROVE);
    assert!(
        visit
            .status
            .is_some_and(|status| (200..300).contains(&status)),
        "{visit:?}"
    );
    let finished = run.finish();
    assert!(finished.success, "{finished:?}");
    assert!(finished.shown().contains(LISTED), "{finished:?}");
}
