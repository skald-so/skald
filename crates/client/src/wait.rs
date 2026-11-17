//! `skald recv --wait`: it shows what waits and exits; with nothing waiting, it waits for the first
//! to come, shows it and exits, or waits until interrupted. As an agent it waits for a message
//! handed over to it; as the person, for anything the person's `skald recv` shows.
//!
//! - Through the helper, such a command is a session of it ([`Sessions`]): the helper runs
//!   `skald recv` for it at once, and again after another command ran. Whenever something comes
//!   that may concern a session, the helper takes it in once for all of them ([`crate::intake`])
//!   and hands each its own, until the session has what it waits for. What a session is handed
//!   goes to its command, so a message is marked handed over only once the command has written it
//!   out; the command exits after.
//! - In one-shot mode it holds its own connection and takes the local state's lock only for each
//!   run of `skald recv`, so a run beside it waits for none of its waits ([`here`]). Once a helper
//!   runs, it goes on through it, as any command does ([`helper::route`]): it ends when the helper
//!   is newer.
//! - Between runs it watches the connection kept: a WebSocket or an event stream; with none, a
//!   long-poll that waits, in the background ([`Watch`]). Off WebSocket, it tries WebSocket again
//!   after an interval ([`RETRY`]), and then the other transports in order.

use std::collections::BTreeSet;
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use skald_proto::SpaceId;

use crate::Operation;
use crate::helper::{self, Route};
use crate::intake::{self, Said};
use crate::lock::Lock;
use crate::mailbox::{self, Came, Transport, Watched};
use crate::recv::{self, Got};

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
    /// What came that the connection did not give: what the long-poll in the background got, or
    /// what a run dropped.
    polled: Arc<Mutex<Polled>>,
    /// The long-poll in the background, while no connection is kept: what stops it, and its
    /// thread.
    poll: Option<(Arc<AtomicBool>, JoinHandle<()>)>,
    /// When to try WebSocket again, while off it.
    retry_at: Option<Instant>,
    /// Whether that time came since this was last asked.
    retried: bool,
}

impl Watch {
    /// Watches from the end of a run of `skald recv --wait` or an intake on this thread: the
    /// connection it kept, or, with none, a long-poll in the background. What the run dropped of
    /// what came counts as come.
    pub fn start(&mut self) {
        let kept = mailbox::kept_transport();
        if mailbox::missed() {
            self.polled.lock().expect("what came").came = Came::Anything;
        }
        self.retry_at = match kept.or_else(intake::left_transport) {
            Some(Transport::WebSocket) => None,
            _ => self.retry_at.or_else(|| Some(Instant::now() + retry())),
        };
        if kept.is_some() {
            self.stop_poll();
        } else if !self.polling()
            && let Some(watched) = intake::left_watched()
        {
            self.poll = Some(poll(watched, Arc::clone(&self.polled)));
        }
    }

    /// What came since: what the long-poll in the background got, or on the connection kept;
    /// anything once a run dropped what came, the connection ended or it is time to try WebSocket
    /// again, which closes the connection kept. Never waits.
    pub fn came(&mut self) -> Came {
        if self.retry_at.is_some_and(|at| Instant::now() >= at) {
            self.retry_at = None;
            self.retried = true;
            self.stop_poll();
            mailbox::close_kept();
            return Came::Anything;
        }
        let polled = std::mem::take(&mut self.polled.lock().expect("what came").came);
        if polled != Came::Nothing {
            return polled;
        }
        if self.polling() {
            return Came::Nothing;
        }
        // With neither, nothing watches: the next run sets the watch again.
        mailbox::kept_came()
    }

    /// The spaces of the `space-notice`s the long-poll in the background got since this was last
    /// asked, which the connection does not give again.
    pub fn noticed(&mut self) -> BTreeSet<SpaceId> {
        std::mem::take(&mut self.polled.lock().expect("what came").noticed)
    }

    /// Whether it was time to try WebSocket again since this was last asked.
    pub fn retried(&mut self) -> bool {
        std::mem::take(&mut self.retried)
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

/// What a long-poll in the background got, and the spaces of the `space-notice`s among it.
#[derive(Default)]
struct Polled {
    came: Came,
    noticed: BTreeSet<SpaceId>,
}

/// Long-polls with `watched` in the background until something comes, which it adds to `polled`,
/// or one fails, which counts as anything, or until stopped; gives what stops it, and its thread.
fn poll(mut watched: Watched, polled: Arc<Mutex<Polled>>) -> (Arc<AtomicBool>, JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            // A failure, too, has the run look again, and say why if it fails.
            let got = watched.poll(POLL_WAIT).unwrap_or(Got::Nothing);
            if got == Got::End {
                continue;
            }
            let mut polled = polled.lock().expect("what came");
            polled.came.add(&got);
            if let Got::Entries(entries) = &got {
                polled.noticed.extend(recv::noticed(entries));
            }
            return;
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
        let ran = helper::run_for(session, dir, crate::received);
        let panicked = self.again(ran, recv::last_space());
        self.watch.start();
        !panicked
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        for waiting in &mut self.waiting {
            waiting.due = true;
        }
    }

    /// Drops the sessions whose command went away; runs `skald recv` again, with the state in
    /// `dir`, for each other one that a command ran since, and takes in what came for each one it
    /// may concern, handing each its own. Gives whether a session waits, `None` once a run or the
    /// intake panicked.
    pub fn look(&mut self, dir: &Path) -> Option<bool> {
        self.waiting.retain(|waiting| open(&waiting.session.stream));
        if self.waiting.is_empty() {
            self.watch = Watch::default();
            // Only drains it.
            mailbox::kept_came();
            return Some(false);
        }
        let came = self.watch.came();
        let (noticed, retry) = (self.watch.noticed(), self.watch.retried());
        let (mut runs, mut handed) = (Vec::new(), Vec::new());
        for waiting in std::mem::take(&mut self.waiting) {
            if waiting.due {
                runs.push(waiting);
            } else if came.concerns(waiting.space) {
                handed.push(waiting);
            } else {
                self.waiting.push(waiting);
            }
        }
        if runs.is_empty() && handed.is_empty() {
            return Some(true);
        }
        let mut panicked = false;
        for waiting in runs {
            let ran = helper::run_for(waiting.session, dir, crate::received);
            panicked |= self.again(ran, recv::last_space());
        }
        panicked |= self.hand(handed, dir, (&noticed, retry));
        if panicked {
            return None;
        }
        self.watch.start();
        Some(!self.waiting.is_empty())
    }

    /// Takes in once what came for `handed`, sessions with the state in `dir`, with the spaces of
    /// the notices that came, `noticed`, trying WebSocket first again after a `retry`, and hands
    /// each its own; with nothing to take in with, runs `skald recv` for each. Whether the intake
    /// or a session's handover panicked.
    fn hand(
        &mut self,
        handed: Vec<Waiting>,
        dir: &Path,
        (noticed, retry): (&BTreeSet<SpaceId>, bool),
    ) -> bool {
        if handed.is_empty() {
            return false;
        }
        let Some(mut intake) = intake::take() else {
            let mut panicked = false;
            for waiting in handed {
                let ran = helper::run_for(waiting.session, dir, crate::received);
                panicked |= self.again(ran, recv::last_space());
            }
            return panicked;
        };
        let scopes = handed.iter().map(|waiting| waiting.space).collect();
        let taken =
            panic::catch_unwind(AssertUnwindSafe(|| intake.take_in(&scopes, noticed, retry)));
        let mut panicked = false;
        match taken {
            Ok(Ok(told)) => {
                let none = Said::new();
                for waiting in handed {
                    let space = waiting.space;
                    let said = match space {
                        Some(space) => told.spaces.get(&space).unwrap_or(&none),
                        None => &told.person,
                    };
                    let ran = helper::run_for(waiting.session, dir, |operation| {
                        let agent = operation.agent()?;
                        intake.hand(said, space.zip(agent.as_ref()))
                    });
                    panicked |= self.again(ran, space);
                }
            }
            Ok(Err(err)) => {
                for waiting in handed {
                    helper::run_for(waiting.session, dir, |_| Err(err.clone()));
                }
            }
            Err(_) => panicked = true,
        }
        intake::leave(intake);
        panicked
    }

    /// Keeps the session of `ran` waiting, if it waits, with the space it kept to, `space`;
    /// whether it panicked.
    fn again(&mut self, ran: Ran, space: Option<SpaceId>) -> bool {
        match ran {
            Ran::Waits(session) => {
                self.waiting.push(Waiting {
                    session: *session,
                    space,
                    due: false,
                });
                false
            }
            Ran::Ended => false,
            Ran::Panicked => true,
        }
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
