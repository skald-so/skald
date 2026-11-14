//! The browser of the browser tests: Chrome Headless Shell, or `SKALD_CHROME`, one per test binary
//! and shared by its tests, each page a browser context of its own. It exits when the test binary
//! or the browser's driver ends, however they end.

mod common;

use std::env;
use std::fs;
use std::thread;

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

/// Killed processes: SIGKILL, so that they do nothing on their way out.
#[cfg(unix)]
mod killed {
    use std::env;
    use std::fs;
    use std::io::{self, BufRead, BufReader, Read};
    use std::path::PathBuf;
    use std::process::{self, Child, Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::common::browser::{Browser, Processes};

    #[test]
    fn the_browser_exits_when_its_test_binary_is_killed() {
        play();
        let (mut binary, processes) =
            binary_with_a_browser("killed::the_browser_exits_when_its_test_binary_is_killed");
        // SIGKILL: the binary does nothing on its way out.
        binary.child.kill().expect("kill the binary");
        binary.child.wait().expect("the binary");
        assert_exit(&[&[processes.driver][..], &processes.browser].concat());
    }

    #[test]
    fn the_browser_exits_when_its_driver_is_killed() {
        play();
        let (_binary, processes) =
            binary_with_a_browser("killed::the_browser_exits_when_its_driver_is_killed");
        let killed = Command::new("kill")
            .args(["-KILL", &processes.driver.to_string()])
            .status()
            .expect("run kill");
        assert!(killed.success(), "kill the driver: {killed}");
        assert_exit(&processes.browser);
    }

    /// Set in a run of this binary that plays a test binary with a browser.
    const PLAY: &str = "SKALD_TEST_BROWSER_BINARY";
    /// What starts the line where that run prints its browser's processes.
    const PROCESSES: &str = "browser processes: ";

    /// This binary running only `test`, which plays a test binary with a browser; killed on drop,
    /// which removes its temporary directory too.
    struct Binary {
        child: Child,
        /// Its TMPDIR, where the browser keeps its profile: a killed driver leaves it behind.
        tmp: PathBuf,
    }

    impl Drop for Binary {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_dir_all(&self.tmp);
        }
    }

    /// Runs `test` of this binary, which calls [`play`], as a test binary with a browser; gives it and
    /// its browser's processes.
    fn binary_with_a_browser(test: &str) -> (Binary, Processes) {
        static BINARIES: AtomicUsize = AtomicUsize::new(0);
        let binary = BINARIES.fetch_add(1, Ordering::Relaxed);
        let tmp = env::temp_dir().join(format!("skald-browser-{}-{binary}", process::id()));
        fs::create_dir(&tmp).expect("the binary's temporary directory");
        let mut child = Command::new(env::current_exe().expect("this test binary"))
            .args([test, "--exact", "--nocapture"])
            .env(PLAY, "1")
            .env("TMPDIR", &tmp)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run this test binary");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let binary = Binary { child, tmp };
        let line = stdout
            .lines()
            .map_while(Result::ok)
            .find_map(|line| line.strip_prefix(PROCESSES).map(str::to_owned))
            .expect("the binary did not print its browser's processes");
        (binary, serde_json::from_str(&line).expect("processes"))
    }

    /// In the run of [`binary_with_a_browser`], starts the browser, prints its processes and waits
    /// until stdin closes, when the test that runs it is gone; then exits. Elsewhere, does nothing.
    fn play() {
        if env::var_os(PLAY).is_none() {
            return;
        }
        let browser = Browser::launch();
        let processes = serde_json::to_string(&browser.processes()).expect("processes");
        println!("{PROCESSES}{processes}");
        let _ = io::stdin().read_to_end(&mut Vec::new());
        process::exit(0);
    }

    /// Waits up to 10 s until none of `pids` runs.
    fn assert_exit(pids: &[u32]) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let running: Vec<u32> = pids.iter().copied().filter(|&pid| runs(pid)).collect();
            if running.is_empty() {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "still running after 10 s: {running:?} of {pids:?}"
            );
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
