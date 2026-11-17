//! `skald recv --wait`: it shows what waits and exits; with nothing waiting, it waits for the first
//! to come, shows it and exits, or waits until interrupted. As an agent it waits for a message
//! handed over to it; as the person, for anything the person's `skald recv` shows.
//!
//! - Through the helper, such a command is a session of it ([`Sessions`]): the helper runs
//!   `skald recv` for it at once, and again whenever something comes that may concern it or another
//!   command ran, until a run shows what the session waits for. Each run writes to the session's command, so it marks
//!   a message handed over only once the command has written it out; the command exits after.
//! - In one-shot mode it holds its own connection and takes the local state's lock only for each
//!   run of `skald recv`, so a run beside it waits for none of its waits ([`here`]). Once a helper
//!   runs, it goes on through it, as any command does ([`helper::route`]): it ends when the helper
//!   is newer.
//! - Between runs it watches the connection kept: a WebSocket or an event stream; with none, a
//!   long-poll that waits, in the background ([`Watch`]). Off WebSocket, it tries WebSocket again
//!   after an interval ([`RETRY`]), and then the other transports in order.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use skald_proto::SpaceId;

use crate::Operation;
use crate::helper::{self, Route};
use crate::lock::Lock;
use crate::mailbox::{self, Came, Transport, Watched};
use crate::recv;

/// How long a connection on SSE or long-poll waits before it tries WebSocket again.
pub const RETRY: Duration = Duration::from_secs(60);

/// How long a long-poll that watches asks the server to wait; the server waits at most its own
/// longest wait.
const POLL_WAIT: Duration = Duration::from_secs(30);

/// How often a one-shot wait looks for what came.
const TICK: Duration = Duration::from_millis(50);

/// The interval this run set, if any.
static RETRY_SET: OnceLock<Duration> = OnceLock::new();

/// Sets how long a connection on SSE or long-poll waits before it tries WebSocket again, for this
/// run and the helper it starts.
pub fn set_retry(retry: Duration) {
    let _ = RETRY_SET.set(retry);
}

/// The interval this run set, if any: a helper it starts takes it.
pub fn retry_set() -> Option<Duration> {
    RETRY_SET.get().copied()
}

/// How long a connection on SSE or long-poll waits before it tries WebSocket again.
fn retry() -> Duration {
    retry_set().unwrap_or(RETRY)
}

/// Runs `operation`, a `skald recv --wait` in one-shot mode with the state in `dir`, holding
/// `lock` for its first run of `skald recv`; gives the exit code of a helper that took over.
pub fn here(
    dir: &Path,
    operation: &Operation,
    lock: Option<Lock>,
) -> Result<Option<ExitCode>, String> {
    let mut first = Some(lock);
    let mut watch = Watch::default();
    loop {
        let lock = match first.take() {
            Some(lock) => lock,
            None => match helper::route(dir, operation, false, false)? {
                Route::Here(lock) => lock,
                Route::Helped(code) => return Ok(Some(code)),
            },
        };
        let shown = crate::received(operation);
        drop(lock);
        if shown? {
            return Ok(None);
        }
        watch.start();
        loop {
            thread::sleep(TICK);
            if helper::running(dir) {
                // It goes on through the helper, on the helper's connection.
                mailbox::close_kept();
                break;
            }
            if watch.came() != Came::Nothing {
                break;
            }
        }
    }
}

/// How a run that waits watches for what comes between its runs of `skald recv`.
#[derive(Default)]
pub struct Watch {
    /// Set once a long-poll in the background got something, or failed.
    came: Arc<AtomicBool>,
    /// The long-poll in the background, while no connection is kept: what stops it, and its
    /// thread.
    poll: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
    /// When to try WebSocket again, while off it.
    retry_at: Option<Instant>,
}

impl Watch {
    /// Watches from the end of a run of `skald recv --wait` on this thread: the connection it kept,
    /// or, with none, a long-poll in the background. What the run dropped of what came counts as
    /// come.
    pub fn start(&mut self) {
        let watched = mailbox::take_watched();
        let kept = mailbox::kept_transport();
        if mailbox::missed() {
            self.came.store(true, Ordering::SeqCst);
        }
        let transport = kept.or(watched.as_ref().map(|watched| watched.transport));
        self.retry_at = match transport {
            Some(Transport::WebSocket) => None,
            _ => self.retry_at.or_else(|| Some(Instant::now() + retry())),
        };
        if kept.is_some() {
            self.stop_poll();
        } else if !self.polling()
            && let Some(watched) = watched
        {
            self.poll = Some(poll(watched, Arc::clone(&self.came)));
        }
    }

    /// What came since: on the connection kept, or anything once the long-poll got something,
    /// the connection ended or it is time to try WebSocket again, which closes the connection kept.
    /// Never waits.
    pub fn came(&mut self) -> Came {
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            self.stop_poll();
            mailbox::close_kept();
            return Came::Anything;
        }
        if self.came.swap(false, Ordering::SeqCst) {
            return Came::Anything;
        }
        if self.polling() {
            return Came::Nothing;
        }
        // With neither, nothing watches: the next run sets the watch again.
        mailbox::kept_came()
    }

    /// Whether a long-poll in the background watches.
    fn polling(&self) -> bool {
        (self.poll.as_ref()).is_some_and(|(_, thread)| !thread.is_finished())
    }

    /// Stops the long-poll in the background, if any, after its request.
    fn stop_poll(&mut self) {
        if let Some((stop, _)) = self.poll.take() {
            stop.store(true, Ordering::SeqCst);
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop_poll();
    }
}

/// Long-polls with `watched` in the background until something comes or one fails, then sets
/// `came`, or until stopped; gives what stops it, and its thread.
fn poll(mut watched: Watched, came: Arc<AtomicBool>) -> (Arc<AtomicBool>, JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            // A failure, too, has the run look again, and say why if it fails.
            if watched.poll(POLL_WAIT).unwrap_or(true) {
                came.store(true, Ordering::SeqCst);
                return;
            }
        }
    });
    (stop, thread)
}

/// The helper's sessions: the `skald recv --wait` commands that wait through it.
#[derive(Default)]
pub struct Sessions {
    waiting: Vec<Waiting>,
    watch: Watch,
}

/// A waiting session, the space its last run kept to, if any, and whether a command ran since:
/// it may have written what the session waits for.
struct Waiting {
    session: Session,
    space: Option<SpaceId>,
    due: bool,
}

/// A session: the command's connection to the helper, its working directory and its operation.
pub struct Session {
    pub stream: helper::Stream,
    pub cwd: PathBuf,
    pub operation: Operation,
}

/// What a session's run of `skald recv` gave.
pub enum Ran {
    /// The session waits on.
    Waits(Box<Session>),
    /// The session ended: it showed what it waits for, or failed.
    Ended,
    /// The run panicked: the helper exits.
    Panicked,
}

impl Sessions {
    /// Runs `session`'s `skald recv` with the state in `dir`; it waits on unless the run showed what
    /// it waits for or failed. Whether the helper goes on.
    pub fn start(&mut self, session: Session, dir: &Path) -> bool {
        let panicked = match helper::run_session(session, dir) {
            Ran::Waits(session) => {
                self.waiting.push(Waiting {
                    session: *session,
                    space: recv::last_space(),
                    due: false,
                });
                false
            }
            Ran::Ended => false,
            Ran::Panicked => true,
        };
        self.watch.start();
        !panicked
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        for waiting in &mut self.waiting {
            waiting.due = true;
        }
    }

    /// Drops the sessions whose command went away, and runs `skald recv` again, with the state in
    /// `dir`, for each other one that what came may concern, or that a command ran since. Gives
    /// whether a session waits, `None` once a run panicked.
    pub fn look(&mut self, dir: &Path) -> Option<bool> {
        self.waiting.retain(|waiting| open(&waiting.session.stream));
        if self.waiting.is_empty() {
            self.watch = Watch::default();
            // Only drains it.
            mailbox::kept_came();
            return Some(false);
        }
        let came = self.watch.came();
        let runs = |waiting: &Waiting| waiting.due || came.concerns(waiting.space);
        if !self.waiting.iter().any(runs) {
            return Some(true);
        }
        let mut panicked = false;
        for waiting in std::mem::take(&mut self.waiting) {
            if !runs(&waiting) {
                self.waiting.push(waiting);
                continue;
            }
            match helper::run_session(waiting.session, dir) {
                Ran::Waits(session) => self.waiting.push(Waiting {
                    session: *session,
                    space: recv::last_space(),
                    due: false,
                }),
                Ran::Ended => {}
                Ran::Panicked => panicked = true,
            }
        }
        if panicked {
            return None;
        }
        self.watch.start();
        Some(!self.waiting.is_empty())
    }
}

/// Whether the command on `stream` is still there: a waiting session's command says nothing, so
/// its connection reads nothing until it ends.
fn open(stream: &helper::Stream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return false;
    }
    let mut byte = [0];
    let waits = matches!(
        (&mut &*stream).read(&mut byte),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock
    );
    stream.set_nonblocking(false).is_ok() && waits
}
