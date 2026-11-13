//! The helpers' connections and databases (`common`): a step PostgreSQL fails by resetting the
//! connection is tried again, a few times; any other error is not; a server that fails to start
//! drops the database it made, as one that stops does.

#[allow(dead_code)]
mod common;

use std::cell::Cell;
use std::future::{Ready, ready};
use std::io;
use std::panic::{self, AssertUnwindSafe};

use common::{SETUP_ATTEMPTS, Server, retried_on_reset, test_databases};

/// An I/O error of `kind`, as sqlx gives it.
fn io_error(kind: io::ErrorKind) -> sqlx::Error {
    sqlx::Error::Io(io::Error::from(kind))
}

/// What [`retried_on_reset`] gives for `step`, whose result is ready at once.
fn retried<T>(step: impl FnMut() -> Ready<Result<T, sqlx::Error>>) -> Result<T, sqlx::Error> {
    tokio::runtime::Runtime::new()
        .expect("start a tokio runtime")
        .block_on(retried_on_reset("a step", step))
}

#[test]
fn only_a_reset_is_retried_and_only_a_few_times() {
    // Resets, then success.
    let calls = Cell::new(0);
    let result = retried(|| {
        calls.set(calls.get() + 1);
        ready(if calls.get() < 3 {
            Err(io_error(io::ErrorKind::ConnectionReset))
        } else {
            Ok(calls.get())
        })
    });
    assert_eq!(result.ok(), Some(3));

    // Resets only: the last one is the error.
    let calls = Cell::new(0);
    let result: Result<(), _> = retried(|| {
        calls.set(calls.get() + 1);
        ready(Err(io_error(io::ErrorKind::ConnectionReset)))
    });
    assert_eq!(calls.get(), SETUP_ATTEMPTS);
    assert!(
        matches!(&result, Err(sqlx::Error::Io(err)) if err.kind() == io::ErrorKind::ConnectionReset),
        "{result:?}"
    );

    // Any other error at once.
    let others: [fn() -> sqlx::Error; 4] = [
        || io_error(io::ErrorKind::ConnectionRefused),
        || io_error(io::ErrorKind::TimedOut),
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
