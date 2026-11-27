//! The helpers' connections and databases (`common`): a step PostgreSQL fails by resetting the
//! connection, or by opening none in time, or that Windows fails with WSAENOBUFS, is tried again, a
//! few times, each retry a line with its error, for WSAENOBUFS with the sampled ports in use; any
//! other error is not; a stand-in's connect, such as the relay's to the server, is tried again
//! the same way while Windows fails it with WSAENOBUFS;
//! a server that fails to start drops the database it made, as one that stops does.

#[allow(dead_code)]
mod common;

use std::cell::{Cell, RefCell};
use std::future::{Ready, ready};
use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::time::{Duration, Instant};

use common::{
    NO_BUFFER_SPACE, SETUP_ATTEMPTS, Server, close, connect, retried_into, server, test_databases,
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
        .block_on(retried_into("a step", None, &mut io::sink(), step))
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
        retried_into("a step", None, &mut io::sink(), || {
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
    let mut sink = Vec::new();
    let calls = Cell::new(0);
    let result = tokio::runtime::Runtime::new()
        .expect("start a tokio runtime")
        .block_on(retried_into("a step", samples, &mut sink, || {
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
             ports: 10:00:10: 950 ports in use"
        ))
    );
    assert_eq!(gone.len(), 2, "{gone:?}");
    assert!(
        gone.iter()
            .all(|line| line.ends_with("; ports: not sampled")),
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
            Some(samples.clone()),
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
             ports: 10:00:10: 950 ports in use"
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
