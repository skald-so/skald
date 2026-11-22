//! Opt-in timing marks for finding where a delay goes: with [`SETTING`] naming a file, each mark
//! appends a line to it: the Unix time in microseconds, the process ID and what happened. Without
//! it, marks cost one check. A span says too how many statements of the state ran within it and
//! how long they took; a statement of [`SLOW_SQL`] or more is marked with its SQL.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::trace::{TraceEvent, TraceEventCodes};

/// The setting that names the file the marks go to.
pub const SETTING: &str = "SKALD_TIMING_LOG";

/// The file the marks go to, if [`SETTING`] names one that opens.
fn log() -> Option<&'static Mutex<File>> {
    static LOG: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os(SETTING).filter(|path| !path.is_empty())?;
        let file = OpenOptions::new().create(true).append(true).open(path);
        file.ok().map(Mutex::new)
    })
    .as_ref()
}

/// How long a statement of the state takes to be marked with its SQL.
const SLOW_SQL: Duration = Duration::from_millis(5);

/// How many statements of the state ran, and their nanoseconds, in this process.
static STATEMENTS: AtomicU64 = AtomicU64::new(0);
static STATEMENT_NANOS: AtomicU64 = AtomicU64::new(0);

/// Times the statements of `db`, a connection to the state, with [`SETTING`].
pub fn watch(db: &rusqlite::Connection) {
    if log().is_some() {
        db.trace_v2(TraceEventCodes::SQLITE_TRACE_PROFILE, Some(statement));
    }
}

/// Counts a statement that ran, marking a slow one with its SQL, without its values.
fn statement(event: TraceEvent<'_>) {
    let TraceEvent::Profile(statement, took) = event else {
        return;
    };
    STATEMENTS.fetch_add(1, Ordering::Relaxed);
    let nanos = u64::try_from(took.as_nanos()).unwrap_or(u64::MAX);
    STATEMENT_NANOS.fetch_add(nanos, Ordering::Relaxed);
    if took >= SLOW_SQL {
        let sql: String = statement
            .sql()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let sql: String = sql.chars().take(120).collect();
        mark(|| format!("sql took {:.1} ms: {sql}", took.as_secs_f64() * 1_000.0));
    }
}

/// The statements that ran so far and their nanoseconds.
fn statements() -> (u64, u64) {
    (
        STATEMENTS.load(Ordering::Relaxed),
        STATEMENT_NANOS.load(Ordering::Relaxed),
    )
}

/// Microseconds since the Unix epoch.
fn now() -> u128 {
    (SystemTime::now().duration_since(UNIX_EPOCH)).map_or(0, |since| since.as_micros())
}

/// Appends `what` at `at`, in microseconds since the Unix epoch.
fn write(at: u128, what: &str) {
    if let Some(log) = log()
        && let Ok(mut file) = log.lock()
    {
        let _ = writeln!(file, "{at} {} {what}", std::process::id());
    }
}

/// Marks that what `what` gives happened now.
pub fn mark(what: impl FnOnce() -> String) {
    if log().is_some() {
        write(now(), &what());
    }
}

/// Marks `name` with its start and how long it took, once dropped, if it took at least `least`
/// milliseconds.
pub struct Span {
    name: &'static str,
    at: u128,
    started: Instant,
    least: u128,
    statements: (u64, u64),
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.started.elapsed().as_micros();
        if took >= self.least * 1_000 {
            let (count, nanos) = statements();
            let (count, nanos) = (count - self.statements.0, nanos - self.statements.1);
            write(
                self.at,
                &format!(
                    "{} took {:.1} ms; sql: {count} statements, {:.1} ms",
                    self.name,
                    took as f64 / 1_000.0,
                    nanos as f64 / 1_000_000.0
                ),
            );
        }
    }
}

/// A [`Span`] of `name` from now, marked whatever it takes; `None` without [`SETTING`].
pub fn span(name: &'static str) -> Option<Span> {
    span_over(name, 0)
}

/// A [`Span`] of `name` from now, marked if it takes at least `least` milliseconds; `None` without
/// [`SETTING`].
pub fn span_over(name: &'static str, least: u128) -> Option<Span> {
    log()?;
    Some(Span {
        name,
        at: now(),
        started: Instant::now(),
        least,
        statements: statements(),
    })
}

/// What `run` gives, marked as a [`Span`] of `name`.
pub fn timed<T>(name: &'static str, run: impl FnOnce() -> T) -> T {
    let _span = span(name);
    run()
}
