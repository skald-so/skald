//! The browser of the browser tests: Chrome Headless Shell, or `SKALD_CHROME`, one per test binary
//! and shared by its tests, each page a browser context of its own. It exits when the test binary
//! or the browser's driver ends, however they end.

mod common;

use std::env;
use std::fs;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use common::browser::{Browser, Processes};
use serde_json::Value;

#[test]
fn the_tests_of_a_binary_share_one_browser() {
    // Each in a thread of its own, as the tests run.
    let shares: Vec<_> = (0..4)
        .map(|_| thread::spawn(|| Browser::launch().processes()))
        .collect();
    let processes: Vec<Processes> = shares
        .into_iter()
        .map(|share| share.join().expect("a share of the browser"))
        .collect();
    let first = &processes[0];
    for other in &processes[1..] {
        assert_eq!(
            (other.driver, other.browser[0]),
            (first.driver, first.browser[0]),
            "another driver or browser: {processes:?}"
        );
    }
}

#[test]
fn each_page_is_a_browser_of_its_own() {
    let browser = Browser::launch();
    let other = Browser::launch();
    let url = format!("http://skald.test:{}/", browser.echo(false));
    let pages = [browser.page(), browser.page(), other.page()];
    for page in &pages {
        page.goto(&url);
    }
    pages[0].evaluate("document.cookie = 'who=first'; localStorage.setItem('who', 'first')");
    assert_eq!(pages[0].cookies_of("skald.test").len(), 1);
    for page in &pages[1..] {
        assert_eq!(page.cookies(), []);
        assert_eq!(page.evaluate("localStorage.getItem('who')"), Value::Null);
    }
}

#[test]
fn a_dropped_share_closes_its_pages_and_servers() {
    let other = Browser::launch();
    let pings = other.echo(false);
    let share = Browser::launch();
    let own = share.echo(false);
    let page = share.page();
    page.goto(&format!("http://skald.test:{own}/"));
    page.evaluate(&format!(
        "setInterval(() => fetch('http://skald.test:{pings}/ping', {{ mode: 'no-cors' }}), 50)"
    ));
    other.wait_for_hits(pings, "/ping", 2);
    drop(share);
    assert!(
        TcpStream::connect(("127.0.0.1", own)).is_err(),
        "its echo server still listens"
    );
    // A ping already on its way may still arrive.
    thread::sleep(Duration::from_millis(500));
    let before = other.hits(pings).len();
    thread::sleep(Duration::from_secs(1));
    assert_eq!(other.hits(pings).len(), before, "its page still runs");
}

#[test]
fn the_browser_is_chrome_headless_shell_or_skald_chrome() {
    let program = Browser::launch().processes().program;
    let program = fs::canonicalize(&program).unwrap_or_else(|err| panic!("{program}: {err}"));
    match env::var_os("SKALD_CHROME") {
        Some(chrome) => assert_eq!(program, fs::canonicalize(chrome).expect("SKALD_CHROME")),
        None => assert_eq!(
            program.file_name(),
            Some("chrome-headless-shell".as_ref()),
            "{program:?}"
        ),
    }
}

/// How the browser ends. A killed process does nothing on its way out: SIGKILL.
#[cfg(unix)]
mod exits {
    use std::env;
    use std::fs;
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::path::{Path, PathBuf};
    use std::process::{self, Child, ChildStdin, Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use serde_json::Value;

    use super::common::browser::{Browser, Processes};

    #[test]
    fn the_browser_exits_when_its_test_binary_is_killed() {
        play();
        let tmp = Tmp::new();
        let (mut binary, processes) = run_playing(
            BINARY,
            "exits::the_browser_exits_when_its_test_binary_is_killed",
            &tmp,
        );
        binary.0.kill().expect("kill the binary");
        binary.0.wait().expect("the binary");
        assert_exit(&[&[processes.driver][..], &processes.browser].concat());
    }

    #[test]
    fn the_driver_exits_when_its_stdin_closes_and_its_browser_with_it() {
        play();
        let tmp = Tmp::new();
        // The run closes the driver's stdin and runs on.
        let (_parent, processes) = run_playing(
            PARENT,
            "exits::the_driver_exits_when_its_stdin_closes_and_its_browser_with_it",
            &tmp,
        );
        assert_exit(&[&[processes.driver][..], &processes.browser].concat());
    }

    #[test]
    fn the_driver_exits_when_its_parent_is_gone_and_its_browser_with_it() {
        let tmp = Tmp::new();
        // Its stdin stays open: this test holds it, not the parent.
        let (mut parent, _stdin, processes) = start_driver(&tmp.0, true);
        parent.0.kill().expect("kill the driver's parent");
        parent.0.wait().expect("the driver's parent");
        assert_exit(&[&[processes.driver][..], &processes.browser].concat());
    }

    #[test]
    fn a_browser_that_would_not_exit_by_itself_dies_with_its_driver() {
        let tmp = Tmp::new();
        let (_driver, _stdin, processes) = start_driver(&tmp.0, false);
        // Stopped, the browser does not exit as its pipe to the driver closes.
        signal("-STOP", processes.browser[0]);
        signal("-KILL", processes.driver);
        let running = running_after_wait(&processes.browser);
        // Nothing else would end them.
        for pid in &running {
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
        assert!(
            running.is_empty(),
            "still running after {WAIT:?}: {running:?}"
        );
    }

    /// How long a process may take to exit, on a busy machine too.
    const WAIT: Duration = Duration::from_secs(30);

    /// A temporary directory of this test, removed on drop: the TMPDIR of a browser it starts, where
    /// the browser keeps its profile, which a killed driver leaves behind.
    struct Tmp(PathBuf);

    impl Tmp {
        fn new() -> Self {
            static TMPS: AtomicUsize = AtomicUsize::new(0);
            let tmp = TMPS.fetch_add(1, Ordering::Relaxed);
            let dir = env::temp_dir().join(format!("skald-browser-{}-{tmp}", process::id()));
            fs::create_dir(&dir).expect("a temporary directory");
            Tmp(dir)
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A process this test started, killed on drop.
    struct Started(Child);

    impl Drop for Started {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    /// Starts the driver with `tmp` for TMPDIR, as this process's child or, with `through_shell`, as
    /// the child of a shell that waits for it; with its stdin from this process either way. Gives
    /// that child, the driver's stdin and its browser's processes.
    fn start_driver(tmp: &Path, through_shell: bool) -> (Started, ChildStdin, Processes) {
        let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let (driver, web) = (tests.join("common/browser.mjs"), tests.join("../../../web"));
        let mut command = if through_shell {
            // A command in the background gets /dev/null for stdin unless redirected.
            let mut shell = Command::new("/bin/sh");
            shell.args(["-c", r#"exec 3<&0; node "$0" "$1" <&3 & wait"#]);
            shell
        } else {
            Command::new("node")
        };
        let mut child = command
            .arg(&driver)
            .arg(&web)
            .env("TMPDIR", tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("start the driver");
        let mut stdin = child.stdin.take().expect("stdin is piped");
        let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
        let started = Started(child);
        let mut line = || {
            lines
                .next()
                .expect("a line of the driver")
                .expect("its stdout")
        };
        assert_eq!(line(), r#"{"ready":true}"#);
        writeln!(stdin, r#"{{"id":1,"session":1,"op":"processes"}}"#).expect("ask the driver");
        let answer: Value = serde_json::from_str(&line()).expect("an answer");
        let processes = serde_json::from_value(answer["ok"].clone()).expect("processes");
        (started, stdin, processes)
    }

    /// What a run of this binary plays, set in its environment: [`BINARY`] or [`PARENT`].
    const PLAY: &str = "SKALD_TEST_BROWSER_PLAY";
    /// A test binary with a browser.
    const BINARY: &str = "binary";
    /// The parent of a driver, the only process it starts, so that no other child holds the
    /// driver's stdin open; it closes that stdin and runs on.
    const PARENT: &str = "parent";
    /// What starts the line where that run prints its browser's processes.
    const PROCESSES: &str = "browser processes: ";

    /// Runs `test` of this binary, which calls [`play`], playing `role` with `tmp` for TMPDIR; gives
    /// that run and its browser's processes.
    fn run_playing(role: &str, test: &str, tmp: &Tmp) -> (Started, Processes) {
        let mut child = Command::new(env::current_exe().expect("this test binary"))
            .args([test, "--exact", "--nocapture"])
            .env(PLAY, role)
            .env("TMPDIR", &tmp.0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run this test binary");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let run = Started(child);
        let line = stdout
            .lines()
            .map_while(Result::ok)
            .find_map(|line| line.strip_prefix(PROCESSES).map(str::to_owned))
            .expect("the run did not print its browser's processes");
        (run, serde_json::from_str(&line).expect("processes"))
    }

    /// In the run of [`run_playing`], plays its role, prints the browser's processes and waits
    /// until stdin closes, when the test that runs it is gone; then exits. Elsewhere, does nothing.
    fn play() {
        let processes = match env::var(PLAY).as_deref() {
            Ok(BINARY) => Browser::launch().processes(),
            Ok(PARENT) => {
                let (mut driver, stdin, processes) = start_driver(&env::temp_dir(), false);
                drop(stdin);
                // Reaped once it exits.
                thread::spawn(move || driver.0.wait());
                processes
            }
            _ => return,
        };
        let processes = serde_json::to_string(&processes).expect("processes");
        println!("{PROCESSES}{processes}");
        let _ = io::stdin().read_to_end(&mut Vec::new());
        process::exit(0);
    }

    /// Sends `signal`, e.g. `-KILL`, to the process `pid`.
    fn signal(signal: &str, pid: u32) {
        let sent = Command::new("kill")
            .args([signal, &pid.to_string()])
            .status()
            .expect("run kill");
        assert!(sent.success(), "kill {signal} {pid}: {sent}");
    }

    /// Waits until none of `pids` runs, for up to [`WAIT`].
    fn assert_exit(pids: &[u32]) {
        let running = running_after_wait(pids);
        assert!(
            running.is_empty(),
            "still running after {WAIT:?}: {running:?} of {pids:?}"
        );
    }

    /// Waits until none of `pids` runs, for up to [`WAIT`]; gives those that still do.
    fn running_after_wait(pids: &[u32]) -> Vec<u32> {
        let deadline = Instant::now() + WAIT;
        loop {
            let running: Vec<u32> = pids.iter().copied().filter(|&pid| runs(pid)).collect();
            if running.is_empty() || Instant::now() >= deadline {
                return running;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Whether the process `pid` runs.
    fn runs(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .expect("run kill")
            .success()
    }
}
