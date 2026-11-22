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
//!   after an interval ([`RETRY`]), and then the other transports in order. In one-shot mode it
//!   also watches the local state for a message another run of the device keeps for it, which no
//!   connection brings.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use skald_proto::SpaceId;

use crate::helper::{self, Route};
use crate::intake::{self, Said};
use crate::lock::Lock;
use crate::mailbox::{self, Came, Transport, Watched};
use crate::mcp::Call;
use crate::recv::{self, Got};
use crate::state::{self, State};
use crate::{Command, Operation, claude_socket, codex, connection, folders, uploads};

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
        // Before the run, so that one kept while it runs counts as come.
        let state = State::open(dir).map_err(|err| err.to_string())?;
        let own = last_own_kept(&state, operation)?;
        let prompt = last_prompt(&state, operation)?;
        let shown = crate::received(operation);
        if operation.agent.is_some() {
            uploads::keep(
                dir,
                uploads::Settings::of(operation),
                &uploads::Live::default(),
            );
        }
        drop(lock);
        if shown? {
            return Ok(None);
        }
        watch.start();
        loop {
            thread::sleep(TICK);
            if helper::running(dir) {
                // It goes on through the helper, on the helper's connection.
                watch.stop_poll();
                mailbox::close_kept();
                break;
            }
            if watch.came() != Came::Nothing
                || last_own_kept(&state, operation)? > own
                || last_prompt(&state, operation)? > prompt
            {
                break;
            }
        }
    }
}

/// The last of the device's own messages that `state` keeps for the run of `operation`
/// ([`State::last_own_kept`]): another run on the device keeps them, and the server does not give
/// them to this one.
fn last_own_kept(state: &State, operation: &Operation) -> Result<u64, String> {
    let last = state.last_own_kept(operation.agent.as_deref());
    last.map_err(|err| err.to_string())
}

/// The last permission prompt `state` records that the person's run has yet to show, for the
/// person's run of `operation`; 0 for an agent's, which shows none ([`crate::permission_prompts`]).
fn last_prompt(state: &State, operation: &Operation) -> Result<u64, String> {
    match operation.agent {
        Some(_) => Ok(0),
        None => state.last_prompt().map_err(|err| err.to_string()),
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
        // A run that ends on WebSocket keeps it.
        self.retry_at = match kept {
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

    /// With no session waiting: closes the connection kept once it is time to try WebSocket again,
    /// while it is an event stream, so the next command's connection tries WebSocket first.
    fn retry_kept(&mut self) {
        if mailbox::kept_transport() != Some(Transport::Sse) {
            self.retry_at = None;
            return;
        }
        let at = *self
            .retry_at
            .get_or_insert_with(|| Instant::now() + retry());
        if Instant::now() >= at {
            self.retry_at = None;
            mailbox::close_kept();
        }
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

/// The helper's sessions: the `skald recv --wait` commands that wait through it, the sessions MCP
/// servers hold of it ([`hold`]), and the Claude Code and Codex sessions it serves
/// ([`claude_socket`], [`codex`]).
#[derive(Default)]
pub struct Sessions {
    waiting: Vec<Waiting>,
    watch: Watch,
    claude: Option<claude_socket::Served>,
    codex: Option<codex::Served>,
    /// When the last `skald recv --wait` of each agent that ended did, as an [`Instant`] and in
    /// Unix seconds.
    ends: BTreeMap<uploads::Running, (Instant, u64)>,
    /// What the presence was last checked with.
    kept: uploads::Kept,
    /// The last permission prompt the person's run has yet to show, when it was last read.
    prompts: Option<(u64, Instant)>,
}

/// How long an agent whose `skald recv --wait` ended counts as running: it runs the next once it
/// has handled what it got.
pub const LINGER: Duration = Duration::from_secs(30);

/// A waiting session, the space its last run kept to, if any, and whether a command ran since:
/// it may have written what the session waits for.
struct Waiting {
    session: Session,
    space: Option<SpaceId>,
    due: bool,
    /// Until when a held session whose run failed to take in is left be.
    resting: Option<Instant>,
}

/// How long a held session whose run failed to take in is left be.
const REST: Duration = Duration::from_secs(30);

/// How often the helper reads the last permission prompt while the person's session waits.
const PROMPTS_EVERY: Duration = Duration::from_millis(100);

thread_local! {
    /// Whether the last run of a held session on this thread failed to take in.
    static HOLD_FAILED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
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
    /// Serves the registered Claude Code and Codex sessions too, its runs with `settings`.
    pub fn serve_agents(&mut self, settings: claude_socket::Settings) {
        self.claude = Some(claude_socket::Served::new(settings));
        self.codex = Some(codex::Served::new(settings));
    }

    /// Runs `session`'s `skald recv` with the state in `dir`; it waits on unless the run showed what
    /// it waits for or failed. A session an MCP server holds starts its agent's session first
    /// ([`hold`]). Whether the helper goes on.
    pub fn start(&mut self, session: Session, dir: &Path) -> bool {
        let others = self.live().running;
        let (held, agent) = (holds(&session.operation), session.operation.agent.clone());
        let ran = helper::run_for(session, dir, |operation| match held {
            true => hold(operation, Some(&others)),
            false => crate::received(operation),
        });
        let space = recv::last_space();
        let went_on = self.again(ran, space).is_some();
        // A `skald recv --wait` that ended at once ran too.
        if let (false, Some(space), Some(agent)) = (held, space, agent) {
            let this = uploads::Running { space, agent };
            if !self.agents(true).contains(&this) {
                self.ends
                    .insert(this, (Instant::now(), crate::state::unix_now()));
            }
        }
        self.keep_presence(dir, true);
        self.watch.start();
        went_on
    }

    /// The agents whose sessions wait, each in the space it kept to: of `skald recv --wait` only if
    /// `waits_only`.
    fn agents(&self, waits_only: bool) -> BTreeSet<uploads::Running> {
        (self.waiting.iter())
            .filter(|waiting| !waits_only || !holds(&waiting.session.operation))
            .filter_map(|waiting| {
                Some(uploads::Running {
                    space: waiting.space?,
                    agent: waiting.session.operation.agent.clone()?,
                })
            })
            .collect()
    }

    /// Notes the end of the `skald recv --wait` of each agent of `before` whose session no longer
    /// waits; `before` then holds those that do.
    fn ended_since(&mut self, before: &mut BTreeSet<uploads::Running>) {
        let now = self.agents(true);
        for gone in before.difference(&now) {
            (self.ends).insert(gone.clone(), (Instant::now(), crate::state::unix_now()));
        }
        *before = now;
    }

    /// The agents whose sessions run, an ended `skald recv --wait` for [`LINGER`] more, and when
    /// the last `skald recv --wait` of each agent ended.
    fn live(&self) -> uploads::Live {
        let mut running = self.agents(false);
        running.extend(
            (self.ends.iter())
                .filter(|(_, (at, _))| at.elapsed() < LINGER)
                .map(|(agent, _)| agent.clone()),
        );
        let ended = (self.ends.iter())
            .map(|(agent, (_, at))| (agent.clone(), *at))
            .collect();
        uploads::Live { running, ended }
    }

    /// Checks the device's presence by the state in `dir` with the sessions' agents, if `anyway`,
    /// once they changed or once it is due ([`uploads::Kept::check`]).
    pub fn keep_presence(&mut self, dir: &Path, anyway: bool) {
        let live = self.live();
        self.kept.check(dir, &live, anyway);
    }

    /// Ends the sessions, as the helper exits, and checks the presence by the state in `dir` with
    /// their agents `offline`.
    pub fn end(mut self, dir: &Path) {
        let mut live = self.live();
        live.running.clear();
        let now = crate::state::unix_now();
        for agent in self.agents(false) {
            live.ended.insert(agent, now);
        }
        self.waiting.clear();
        self.kept.check(dir, &live, false);
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        for waiting in &mut self.waiting {
            waiting.due = true;
        }
        if let Some(claude) = &mut self.claude {
            claude.ran();
        }
        if let Some(codex) = &mut self.codex {
            codex.ran();
        }
    }

    /// Drops the sessions whose command went away; serves the Claude Code and Codex sessions with
    /// what came ([`claude_socket::Served::look`], [`codex::Served::look`]); runs `skald recv`
    /// again, with the state in `dir`, for each other one that a command ran since, or for each
    /// once a `link-notice` came, and takes in what came for each one it may concern, handing each
    /// its own. Gives whether a session waits or a Claude Code or Codex session is served, which
    /// after a run or a handover the next look tells; `None` once a run or the intake panicked.
    pub fn look(&mut self, dir: &Path) -> Option<bool> {
        let mut before = self.agents(true);
        let looked = self.looked(dir, &mut before);
        self.ended_since(&mut before);
        looked
    }

    /// [`Sessions::look`], noting the sessions of `before` that ended before it takes in again;
    /// checks the presence after a run or an intake.
    fn looked(&mut self, dir: &Path, before: &mut BTreeSet<uploads::Running>) -> Option<bool> {
        self.waiting.retain(|waiting| open(&waiting.session.stream));
        let claude = (self.claude.as_ref()).is_some_and(claude_socket::Served::looks)
            || (self.codex.as_mut()).is_some_and(|codex| codex.looks(dir));
        if self.waiting.is_empty() && !claude {
            let mut idle = Watch::default();
            idle.retry_at = self.watch.retry_at;
            self.watch = idle;
            self.watch.retry_kept();
            // Only drains it.
            mailbox::kept_came();
            mailbox::linking_came();
            return Some(false);
        }
        let came = self.watch.came();
        // Each session's run says the devices that wait for linking.
        if mailbox::linking_came() {
            self.ran();
        }
        self.prompts_came(dir);
        if let Some(claude) = &mut self.claude
            && claude.look(dir, &came)?
        {
            self.watch.start();
        }
        if let Some(codex) = &mut self.codex
            && codex.look(dir, &came)?
        {
            self.watch.start();
        }
        if self.waiting.is_empty() {
            return Some(
                (self.claude.as_ref()).is_some_and(claude_socket::Served::serving)
                    || (self.codex.as_ref()).is_some_and(codex::Served::serving),
            );
        }
        let seen = SystemTime::now();
        let (noticed, retry) = (self.watch.noticed(), self.watch.retried());
        let (mut runs, mut handed) = (Vec::new(), Vec::new());
        let now = Instant::now();
        for waiting in std::mem::take(&mut self.waiting) {
            if waiting.resting.is_some_and(|until| now < until) {
                self.waiting.push(waiting);
            } else if waiting.due {
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
        for waiting in runs {
            let ran = run_again(waiting.session, dir);
            self.again(ran, recv::last_space())?;
        }
        self.hand(handed, dir, (&noticed, retry, seen))?;
        self.ended_since(before);
        self.keep_presence(dir, true);
        self.watch.start();
        // The next look counts the sessions left.
        Some(true)
    }

    /// Has the person's sessions run again once a permission prompt is recorded in the state in
    /// `dir`, which a Codex hook or the helper's client of an app-server records without a command
    /// ([`crate::permission_prompts`]); reads the state at most every [`PROMPTS_EVERY`].
    fn prompts_came(&mut self, dir: &Path) {
        let person = (self.waiting.iter()).any(|waiting| waiting.session.operation.agent.is_none());
        if !person {
            self.prompts = None;
            return;
        }
        if (self.prompts).is_some_and(|(_, read)| read.elapsed() < PROMPTS_EVERY) {
            return;
        }
        let Some(last) = state::last_prompt(dir) else {
            return;
        };
        let before = self.prompts.replace((last, Instant::now()));
        if last > before.map_or(0, |(before, _)| before) {
            for waiting in &mut self.waiting {
                if waiting.session.operation.agent.is_none() {
                    waiting.due = true;
                }
            }
        }
    }

    /// Takes in once what came for `handed`, sessions with the state in `dir`, with the spaces of
    /// the notices that came, `noticed`, trying WebSocket first again after a `retry`, and hands
    /// each its own; with nothing to take in with, runs `skald recv` for each. `None` once the
    /// intake or a session's handover panicked: the others are left to the helper's exit.
    fn hand(
        &mut self,
        handed: Vec<Waiting>,
        dir: &Path,
        (noticed, retry, seen): (&BTreeSet<SpaceId>, bool, SystemTime),
    ) -> Option<()> {
        if handed.is_empty() {
            return Some(());
        }
        let Some(mut intake) = intake::take() else {
            for waiting in handed {
                let ran = run_again(waiting.session, dir);
                self.again(ran, recv::last_space())?;
            }
            return Some(());
        };
        let scopes = handed.iter().map(|waiting| waiting.space).collect();
        let mut marks = vec![("came", seen), ("taking in", SystemTime::now())];
        let taken = panic::catch_unwind(AssertUnwindSafe(|| {
            intake.take_in(&scopes, noticed, (retry, &mut marks))
        }));
        match taken.ok()? {
            Ok(told) => {
                let none = Said::new();
                for waiting in handed {
                    let space = waiting.space;
                    // A held session is handed nothing: its agent's `inbox` takes it.
                    if holds(&waiting.session.operation) {
                        self.waiting.push(waiting);
                        continue;
                    }
                    let said = match space {
                        Some(space) => told.spaces.get(&space).unwrap_or(&none),
                        None => &told.person,
                    };
                    let ran = helper::run_for(waiting.session, dir, |operation| {
                        if operation.command.traces() {
                            trace(&marks);
                        }
                        let agent = operation.agent()?;
                        intake.hand(said, space.zip(agent.as_ref()), operation.json())
                    });
                    self.again(ran, space)?;
                }
                intake.send_receipts();
            }
            Err(err) => {
                for mut waiting in handed {
                    if holds(&waiting.session.operation) {
                        waiting.resting = Some(Instant::now() + REST);
                        self.waiting.push(waiting);
                        continue;
                    }
                    helper::run_for(waiting.session, dir, |_| Err(err.clone()));
                }
            }
        }
        intake::leave(intake);
        Some(())
    }

    /// Keeps the session of `ran` waiting, if it waits, with the space it kept to, `space`;
    /// `None` if it panicked.
    fn again(&mut self, ran: Ran, space: Option<SpaceId>) -> Option<()> {
        match ran {
            Ran::Waits(session) => {
                let failed = holds(&session.operation) && HOLD_FAILED.with(|failed| failed.take());
                self.waiting.push(Waiting {
                    session: *session,
                    space,
                    due: false,
                    resting: failed.then(|| Instant::now() + REST),
                });
                Some(())
            }
            Ran::Ended => Some(()),
            Ran::Panicked => None,
        }
    }
}

/// Runs `skald recv` again for `session` with the state in `dir`, or takes in again for a held
/// one.
fn run_again(session: Session, dir: &Path) -> Ran {
    helper::run_for(session, dir, |operation| match holds(operation) {
        true => hold(operation, None),
        false => crate::received(operation),
    })
}

/// Whether `operation` is the session an MCP server holds of the helper.
pub fn holds(operation: &Operation) -> bool {
    matches!(
        operation.command,
        Command::Mcp {
            call: Some(Call::Hold { .. }),
            ..
        }
    )
}

/// The helper's run for the session an MCP server holds of it, its `operation`: in the working
/// directory's space, it takes in what waits for the session's agent, handing nothing over
/// ([`recv::hold_run`]), and waits on; in no space, it ends. On the session's start, `others` gives
/// the agents of the other sessions: unless one of them is its agent in that space, its agent's
/// session starts ([`State::agent_session_started`]). A failure to take in leaves the session be
/// for [`REST`].
fn hold(
    operation: &Operation,
    others: Option<&BTreeSet<uploads::Running>>,
) -> Result<bool, String> {
    let (
        Some(agent),
        Command::Mcp {
            call: Some(Call::Hold { runtime }),
            ..
        },
    ) = (operation.agent()?, &operation.command)
    else {
        return Err("only an MCP server holds a session of the helper".to_owned());
    };
    crate::clock::set_offset(operation.clock_offset);
    let dir = crate::state::dir(operation.state_dir.clone())?;
    uploads::remember(uploads::Settings::of(operation));
    let Ok(space) = recv::holding_warnings(&dir, true, || folders::working_space(&dir)) else {
        return Ok(true);
    };
    if let Some(others) = others {
        let running = uploads::Running {
            space: space.id,
            agent: agent.to_string(),
        };
        if !others.contains(&running) {
            let held = State::open(&dir).map_err(|err| err.to_string())?;
            (held.add_agent(space.id, agent.as_str()))
                .and_then(|()| {
                    held.agent_session_started(space.id, agent.as_str(), runtime.as_deref())
                })
                .map_err(|err| err.to_string())?;
        }
    }
    let settings = (operation.speaks, operation.timeout);
    let held = recv::hold_run(&dir, operation.key_storage, settings, &agent);
    HOLD_FAILED.with(|failed| failed.set(held.is_err()));
    Ok(false)
}

/// Shows when each of `marks` and the handover to a session started, in milliseconds since the
/// Unix epoch.
fn trace(marks: &intake::Marks) {
    let handing = ("handing over", SystemTime::now());
    for (step, at) in marks.iter().chain([&handing]) {
        let at = at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        eprintln!("skald: trace: {step} at {at}");
    }
}

/// Whether the command on `stream` is still there: a waiting session's command says nothing, so
/// its connection reads nothing until it ends.
fn open(stream: &helper::Stream) -> bool {
    connection::idle(
        |on| stream.set_nonblocking(on),
        |byte| (&mut &*stream).read(byte),
    )
}
