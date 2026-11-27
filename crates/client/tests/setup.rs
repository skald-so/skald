//! The helpers' connections and databases (`common`): a step PostgreSQL fails by resetting the
//! connection, or by opening none in time, or that Windows fails with WSAENOBUFS, is tried again, a
//! few times, each retry a line with its error, for WSAENOBUFS with the sampled ports in use; any
//! other error is not; a stand-in's connect, such as the relay's to the server, is tried again
//! the same way while Windows fails it with WSAENOBUFS;
//! a server that fails to start drops the database it made, as one that stops does; on Windows the
//! browser tests' driver and its browser end with their test binary, and a later driver removes
//! what they left.

#[cfg(windows)]
#[allow(dead_code)]
#[path = "../../server/tests/common/browser.rs"]
mod browser;
#[allow(dead_code)]
mod common;

use std::cell::{Cell, RefCell};
use std::future::{Ready, ready};
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use common::{
    NO_BUFFER_SPACE, SETUP_ATTEMPTS, Samples, Server, close, connect, retried_into, server,
    shows_no_buffer_space, test_databases,
};

/// An I/O error of `kind`, as sqlx gives it.
fn io_error(kind: io::ErrorKind) -> sqlx::Error {
    sqlx::Error::Io(io::Error::from(kind))
}

/// What `retried_on_reset` gives for `step`, whose result is ready at once; its retries' lines are
/// dropped.
fn retried<T>(step: impl FnMut() -> Ready<Result<T, sqlx::Error>>) -> Result<T, sqlx::Error> {
    tokio::runtime::Runtime::new()
        .expect("start a tokio runtime")
        .block_on(retried_into("a step", &noted(None), &mut io::sink(), step))
}

#[test]
fn only_a_reset_a_timeout_or_no_buffer_space_is_retried_and_only_a_few_times() {
    let errors: [fn() -> io::Error; 3] = [
        || io::Error::from(io::ErrorKind::ConnectionReset),
        || io::Error::from(io::ErrorKind::TimedOut),
        || io::Error::from_raw_os_error(NO_BUFFER_SPACE),
    ];
    for error in errors {
        let want = error().to_string();
        // Failures, then success.
        let calls = Cell::new(0);
        let result = retried(|| {
            calls.set(calls.get() + 1);
            ready(if calls.get() < 3 {
                Err(sqlx::Error::Io(error()))
            } else {
                Ok(calls.get())
            })
        });
        assert_eq!(result.ok(), Some(3), "{want}");

        // Failures only: the last one is the error.
        let calls = Cell::new(0);
        let result: Result<(), _> = retried(|| {
            calls.set(calls.get() + 1);
            ready(Err(sqlx::Error::Io(error())))
        });
        assert_eq!(calls.get(), SETUP_ATTEMPTS, "{want}");
        assert!(
            matches!(&result, Err(sqlx::Error::Io(err)) if err.to_string() == want),
            "{result:?}"
        );
    }

    // Any other error at once.
    let others: [fn() -> sqlx::Error; 3] = [
        || io_error(io::ErrorKind::ConnectionRefused),
        || sqlx::Error::PoolTimedOut,
        || sqlx::Error::Protocol("a refusal".to_owned()),
    ];
    for error in others {
        let calls = Cell::new(0);
        let result: Result<(), _> = retried(|| {
            calls.set(calls.get() + 1);
            ready(Err(error()))
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 1, "{result:?}");
    }
}

#[test]
fn a_reset_is_retried_after_waits_of_100_ms_doubled_1_5_s_in_all() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .expect("start a tokio runtime");
    let attempts = RefCell::new(Vec::new());
    let result: Result<(), _> = runtime.block_on(async {
        let started = tokio::time::Instant::now();
        retried_into("a step", &noted(None), &mut io::sink(), || {
            attempts.borrow_mut().push(started.elapsed());
            ready(Err(io_error(io::ErrorKind::ConnectionReset)))
        })
        .await
    });
    assert!(result.is_err());
    let at = [0, 100, 300, 700, 1500].map(Duration::from_millis);
    assert_eq!(attempts.into_inner(), at);
}

/// The lines of the retries of a step that fails twice with `error`, the ports sampled in `samples`.
fn retry_lines(error: fn() -> io::Error, samples: Option<std::path::PathBuf>) -> Vec<String> {
    let samples = noted(samples);
    let mut sink = Vec::new();
    let calls = Cell::new(0);
    let result = tokio::runtime::Runtime::new()
        .expect("start a tokio runtime")
        .block_on(retried_into("a step", &samples, &mut sink, || {
            calls.set(calls.get() + 1);
            ready(if calls.get() < 3 {
                Err(sqlx::Error::Io(error()))
            } else {
                Ok(())
            })
        }));
    assert!(result.is_ok(), "{result:?}");
    String::from_utf8(sink)
        .expect("lines in UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The samples in `file`, the machine's state now always `taken`.
fn noted(file: Option<std::path::PathBuf>) -> Samples {
    Samples {
        file,
        now: || "taken".to_owned(),
    }
}

/// A new directory of this process named for `name`, removed by the test, and its samples file
/// holding `text`, as CI's sampler of the ports in use writes it.
fn sampled(name: &str, text: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("skald-client-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("make a directory");
    let samples = dir.join("ports.txt");
    std::fs::write(&samples, text).expect("write the samples");
    (dir, samples)
}

#[test]
fn a_retry_is_a_line_with_its_error_and_for_no_buffer_space_the_last_sample_of_the_ports() {
    let (dir, samples) = sampled(
        "samples",
        "10:00:00: 900 ports in use\n10:00:10: 950 ports in use\n\n",
    );
    let no_buffer_space = || io::Error::from_raw_os_error(NO_BUFFER_SPACE);
    let sampled = retry_lines(no_buffer_space, Some(samples.clone()));
    let gone = retry_lines(no_buffer_space, Some(dir.join("none.txt")));
    let reset = retry_lines(
        || io::Error::from(io::ErrorKind::ConnectionReset),
        Some(samples),
    );
    std::fs::remove_dir_all(&dir).expect("remove the directory");

    let error = no_buffer_space().to_string();
    assert_eq!(
        sampled,
        [100, 200].map(|ms| format!(
            "a step: no connection to PostgreSQL: {error}; again in {ms}ms; \
             ports: 10:00:10: 950 ports in use; now: taken"
        ))
    );
    assert_eq!(gone.len(), 2, "{gone:?}");
    assert!(
        gone.iter()
            .all(|line| line.ends_with("; ports: not sampled; now: taken")),
        "{gone:?}"
    );
    let reset_error = io::Error::from(io::ErrorKind::ConnectionReset).to_string();
    assert_eq!(
        reset,
        [100, 200].map(|ms| format!(
            "a step: no connection to PostgreSQL: {reset_error}; again in {ms}ms"
        ))
    );
}

#[test]
fn a_stand_ins_connect_is_tried_again_only_while_windows_has_no_buffer_space() {
    let (dir, samples) = sampled("reach", "10:00:10: 950 ports in use\n");
    // Calls of a connect failing with `error` until its call `succeeds`, and the retries' lines.
    let reach = |error: fn() -> io::Error, succeeds: u32| {
        let mut sink = Vec::new();
        let calls = Cell::new(0);
        let started = Instant::now();
        let result = common::reached(
            "a relay",
            "127.0.0.1:1",
            &noted(Some(samples.clone())),
            &mut sink,
            || {
                calls.set(calls.get() + 1);
                if calls.get() < succeeds {
                    Err(error())
                } else {
                    Ok(())
                }
            },
        );
        let lines = String::from_utf8(sink).expect("lines in UTF-8");
        (result, calls.get(), started.elapsed(), lines)
    };
    let no_buffer_space = || io::Error::from_raw_os_error(NO_BUFFER_SPACE);
    let refused = || io::Error::from(io::ErrorKind::ConnectionRefused);
    let (recovered, recovered_calls, recovered_in, recovered_lines) = reach(no_buffer_space, 3);
    let (failed, failed_calls, failed_in, _) = reach(no_buffer_space, u32::MAX);
    let (other, other_calls, _, other_lines) = reach(refused, u32::MAX);
    std::fs::remove_dir_all(&dir).expect("remove the directory");

    let error = no_buffer_space().to_string();
    assert!(recovered.is_ok(), "{recovered:?}");
    assert_eq!(recovered_calls, 3);
    assert!(
        recovered_in >= Duration::from_millis(300),
        "{recovered_in:?}"
    );
    assert_eq!(
        recovered_lines.lines().collect::<Vec<_>>(),
        [100, 200].map(|ms| format!(
            "a relay: cannot reach 127.0.0.1:1: {error}; again in {ms}ms; \
             ports: 10:00:10: 950 ports in use; now: taken"
        ))
    );
    assert!(
        failed.as_ref().is_err_and(|err| err.to_string() == error),
        "{failed:?}"
    );
    assert_eq!(failed_calls, SETUP_ATTEMPTS);
    assert!(failed_in >= Duration::from_millis(1500), "{failed_in:?}");
    assert!(other.is_err_and(|err| err.kind() == io::ErrorKind::ConnectionRefused));
    assert_eq!(other_calls, 1);
    assert_eq!(other_lines, "");
}

/// The only test of this binary that makes databases: no other test's are listed.
#[test]
fn a_server_drops_its_database_also_when_it_fails_to_start() {
    let server = Server::start(None);
    assert_eq!(test_databases().len(), 1);
    drop(server);
    assert_eq!(test_databases(), Vec::<String>::new());

    let failed = panic::catch_unwind(AssertUnwindSafe(|| {
        Server::start_with(|_| panic!("the server's settings fail"))
    }));
    assert!(failed.is_err(), "the server started");
    assert_eq!(test_databases(), Vec::<String>::new());
}

/// PostgreSQL on Windows resets a connection as its session ends, also one that a `Terminate`
/// ends: closing a connection whose session has ended is no failure.
#[test]
fn a_connection_whose_session_ended_closes() {
    let runtime = tokio::runtime::Runtime::new().expect("start a tokio runtime");
    runtime.block_on(async {
        let url = format!("{}/postgres", server());
        let mut ended = connect(&url).await.expect("connect");
        let mut other = connect(&url).await.expect("connect");
        let (pid,): (i32,) = sqlx::query_as("SELECT pg_backend_pid()")
            .fetch_one(&mut ended)
            .await
            .expect("the backend's PID");
        sqlx::query("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .execute(&mut other)
            .await
            .expect("end the session");
        let started = Instant::now();
        loop {
            let (left,): (i64,) =
                sqlx::query_as("SELECT count(*) FROM pg_stat_activity WHERE pid = $1")
                    .bind(pid)
                    .fetch_one(&mut other)
                    .await
                    .expect("the sessions");
            if left == 0 {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the session goes on"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // The reset reaches the connection.
        tokio::time::sleep(Duration::from_millis(200)).await;

        close(ended).await;
        close(other).await;
    });
}

#[test]
fn an_output_shows_no_buffer_space_once_its_new_bytes_end_the_error_also_split_across_reads() {
    let line = b"skald: cannot reach the server: lacked buffer space (os error 10055)\n";
    let split = line.len() - 8;
    assert!(!shows_no_buffer_space(&line[..split], split));
    assert!(shows_no_buffer_space(line, line.len() - split));
    assert!(shows_no_buffer_space(line, line.len()));
    let earlier = [&line[..], b"and then more"].concat();
    assert!(!shows_no_buffer_space(&earlier, 13));
    assert!(!shows_no_buffer_space(b"os error 10054\n", 16));
    assert!(!shows_no_buffer_space(b"", 0));
}

/// The driver of a run of this binary, which cannot exit by itself, and its browser end with that
/// run; the next driver removes the run's directory, with the browser's profile in it.
#[cfg(windows)]
#[test]
fn a_browser_ends_with_its_test_binary_and_a_later_driver_removes_its_profile() {
    use std::io::{BufRead, BufReader, Read};
    use std::process::{Command, Stdio};

    use browser::{Browser, contained};
    use held::Held;

    const TEST: &str = "a_browser_ends_with_its_test_binary_and_a_later_driver_removes_its_profile";
    /// Set in the environment of that run.
    const PLAY: &str = "SKALD_TEST_BROWSER_PLAY";
    /// What starts the line where that run prints the processes in its driver's job.
    const PROCESSES: &str = "browser processes: ";
    if std::env::var_os(PLAY).is_some() {
        let browser = Browser::launch();
        let driver = browser.processes().driver;
        let contained = browser.contained();
        assert!(contained.contains(&driver), "{driver} not in {contained:?}");
        // Its driver no longer sees its stdin close, nor closes the browser.
        contained::suspend(driver).expect("suspend the driver");
        // With when each started, so that the test tells them from later processes of their IDs.
        let started: Vec<(u32, Option<u64>)> = contained
            .iter()
            .filter_map(|&pid| Held::open(pid).map(|process| (pid, process.created())))
            .collect();
        let started = serde_json::to_string(&started).expect("processes");
        println!("{PROCESSES}{started}");
        // It ends once the test, having looked at its directory, closes its stdin: other tests'
        // drivers remove the directory of a binary no longer running.
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        std::process::exit(0);
    }
    let mut run = Command::new(std::env::current_exe().expect("this test binary"))
        .args([TEST, "--exact", "--nocapture"])
        .env(PLAY, "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("run this test binary");
    let stdin = run.stdin.take().expect("stdin is piped");
    let stdout = BufReader::new(run.stdout.take().expect("stdout is piped"));
    let printed = stdout
        .lines()
        .map_while(Result::ok)
        .find_map(|line| line.strip_prefix(PROCESSES).map(str::to_owned));
    let Some(printed) = printed else {
        drop(stdin);
        let status = run.wait().expect("the run");
        panic!("the run, {status}, printed no processes");
    };
    let listed: Vec<(u32, Option<u64>)> = serde_json::from_str(&printed).expect("processes");
    // Held open, a process keeps its ID; one gone, or whose ID went to a later one, has ended.
    let held: Vec<(u32, Held)> = listed
        .iter()
        .filter_map(|&(pid, created)| {
            Held::open(pid)
                .filter(|process| process.created() == created)
                .map(|process| (pid, process))
        })
        .collect();
    // `run` holds its process open until it drops, so its ID, the name of its directory, stays its.
    let dir = contained::dir_of(run.id());
    let filled = std::fs::read_dir(&dir).is_ok_and(|mut entries| entries.next().is_some());
    drop(stdin);
    let status = run.wait().expect("the run");
    let deadline = Instant::now() + Duration::from_secs(30);
    let running = loop {
        let running: Vec<u32> = held
            .iter()
            .filter(|(_, process)| !process.ended())
            .map(|&(pid, _)| pid)
            .collect();
        if running.is_empty() || Instant::now() >= deadline {
            break running;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    // Nothing else would end them; their IDs are still theirs.
    for pid in &running {
        let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
    }
    assert!(
        running.is_empty(),
        "still running: {running:?} of {listed:?}"
    );
    assert!(status.success(), "the run: {status}");
    assert!(filled, "nothing in {}", dir.display());
    let _browser = Browser::launch();
    // Another test's driver may be removing it at the same time.
    let deadline = Instant::now() + Duration::from_secs(5);
    while dir.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!dir.exists(), "{} is left", dir.display());
}

/// A process held open, so that its ID goes to no other process while a test looks at it.
#[cfg(windows)]
mod held {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

    use windows_sys::Win32::Foundation::{FILETIME, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        WaitForSingleObject,
    };

    pub struct Held(OwnedHandle);

    impl Held {
        /// The process `pid`, unless it has ended.
        pub fn open(pid: u32) -> Option<Self> {
            [
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE,
            ]
            .into_iter()
            .find_map(|access| {
                // SAFETY: it takes no pointer; the handle is owned from here on.
                let process = unsafe { OpenProcess(access, 0, pid) };
                // SAFETY: a new handle that nothing else owns.
                (!process.is_null()).then(|| Held(unsafe { OwnedHandle::from_raw_handle(process) }))
            })
            .filter(|held| !held.ended())
        }

        /// When it started, in 100 ns since 1601, if the handle may read it.
        pub fn created(&self) -> Option<u64> {
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let [mut created, mut exited, mut kernel, mut user] = [zero; 4];
            // SAFETY: an open handle; each pointer is to a FILETIME.
            let got = unsafe {
                GetProcessTimes(
                    self.0.as_raw_handle(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
            };
            (got != 0).then(|| {
                (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)
            })
        }

        /// Whether it has ended and closed its files.
        pub fn ended(&self) -> bool {
            // SAFETY: an open handle with the right to wait for it.
            unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) != WAIT_TIMEOUT }
        }
    }
}
