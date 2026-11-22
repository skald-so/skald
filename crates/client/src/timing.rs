//! Opt-in timing marks for finding where a delay goes: with [`SETTING`] naming a file, each mark
//! appends a line to it: the Unix time in microseconds, the process ID and what happened. Without
//! it, marks cost one check.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.started.elapsed().as_micros();
        if took >= self.least * 1_000 {
            write(
                self.at,
                &format!("{} took {:.1} ms", self.name, took as f64 / 1_000.0),
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
    })
}

/// What `run` gives, marked as a [`Span`] of `name`.
pub fn timed<T>(name: &'static str, run: impl FnOnce() -> T) -> T {
    let _span = span(name);
    run()
}
