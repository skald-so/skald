//! Starts the installed CLIs and reads their output; everything else is in the library.

use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::hook::{self, HOOK_BIN, Hook, HookCheck, HookRuns, hook_runs_file};
use agent_live::{
    Agent, BusyMessage, Call, Event, INBOX_BIN, InboxCheck, Kind, Options, Run, Session, Setup,
    Stub, add_message, binary, channel, session_failure, token,
};
use agent_source::{Message, Source, State, now_ms};
use tempfile::TempDir;

use crate::Result;

/// The longest a session may take.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Starts a check of `agent`: prints its CLI's version and makes the check's temporary directory.
pub fn start(agent: Agent) -> Result<TempDir> {
    let version = agent.command().arg("--version").output()?;
    println!("{}", agent.version(&version.stdout));
    Ok(tempfile::Builder::new().prefix("skald-live-").tempdir()?)
}

/// Starts a check of `agent` whose sessions run the workspace's binary `name`, as [`start`] does;
/// returns its temporary directory, the binary, and `live-check`'s `PATH`, the only variable the
/// CLI on the stub inherits.
pub fn start_with(agent: Agent, name: &str) -> Result<(TempDir, PathBuf, String)> {
    let work = start(agent)?;
    let binary = binary(&std::env::current_exe()?, name)?;
    Ok((work, binary, std::env::var("PATH").unwrap_or_default()))
}

/// Ends a check that ended with `pass`: prints its `verdict` and, if `options` keep the logs, where
/// `work` stays. Returns `pass`.
pub fn finish(work: TempDir, options: &Options, verdict: &str, pass: bool) -> bool {
    println!("{verdict}");
    if options.keeps_logs(pass) {
        println!("logs: {}", work.keep().display());
    }
    pass
}

/// Runs session `a` and, if given, session `b` at the same time.
pub fn side_by_side<A, B: Send>(
    a: impl FnOnce() -> A,
    b: Option<impl FnOnce() -> B + Send>,
) -> (A, Option<B>) {
    thread::scope(|scope| {
        let b = b.map(|b| scope.spawn(b));
        (a(), b.map(|b| b.join().expect("session b ran")))
    })
}

/// How a check prepares its sessions and adds their messages.
struct Sessions<'a, P> {
    options: &'a Options<'a>,
    /// Prepares the session of a name and kind on its model stub, if any.
    prepare: P,
    /// Adds a session's message with a token.
    add: fn(&Source, &str) -> io::Result<Message>,
}

/// What the idle sessions of a check ran into.
struct Idle<S> {
    a: S,
    /// The token of a's message.
    token: String,
    /// Where a's message is after a's session.
    state: Option<State>,
    b: Option<Result<S>>,
}

/// What the busy session of a check ran into.
struct Busy {
    run: Run,
    session: Session,
    token: String,
    /// When the message was added and where it is now; neither if the agent ran no shell command.
    added_at_ms: Option<u64>,
    state: Option<State>,
}

impl<P: Fn(&str, Kind, Option<&Stub>) -> io::Result<Run>> Sessions<'_, P> {
    /// Runs the idle sessions: session a, with a message, and, with `--all`, session b, with its
    /// own empty source, at the same time, each on a model stub, if any, playing `script`; `run`
    /// runs a session.
    fn idle<S: Send>(
        &self,
        script: &'static [Call],
        run: impl Fn(&Run, Option<&Stub>) -> Result<S> + Sync,
    ) -> Result<Idle<S>> {
        let stub_a = self.options.stub(script)?;
        let a = (self.prepare)("a", Kind::Idle, stub_a.as_ref())?;
        let source_a = Source::open(&a.source)?;
        let token = token();
        let message = (self.add)(&source_a, &token)?;
        let b = if self.options.all {
            let stub = self.options.stub(script)?;
            Some(((self.prepare)("b", Kind::Idle, stub.as_ref())?, stub))
        } else {
            None
        };
        let (session_a, session_b) = side_by_side(
            || run(&a, stub_a.as_ref()),
            b.as_ref().map(|(b, stub)| || run(b, stub.as_ref())),
        );
        let a = session_a?;
        let state = source_a.state(&message.msg_id)?;
        Ok(Idle {
            a,
            token,
            state,
            b: session_b,
        })
    }

    /// Runs the busy session, c, on a model stub, if any, playing `script`: its message is added
    /// when the agent starts its first shell command; `on_event` gets each event first.
    fn busy(&self, script: &'static [Call], mut on_event: impl FnMut(&Event)) -> Result<Busy> {
        let stub = self.options.stub(script)?;
        let run = (self.prepare)("c", Kind::Busy, stub.as_ref())?;
        let source = Source::open(&run.source)?;
        let token = token();
        let mut message = BusyMessage::default();
        let session = run_session(&run, stub.as_ref(), |event| {
            on_event(event);
            message.on_event(event, || (self.add)(&source, &token));
        })?;
        let (added_at_ms, state) = match message.added().transpose()? {
            Some(message) => (Some(message.added_at_ms), source.state(&message.msg_id)?),
            None => (None, None),
        };
        Ok(Busy {
            run,
            session,
            token,
            added_at_ms,
            state,
        })
    }
}

/// The `inbox` path's live check: an idle session; with `--all`, also the other of two idle
/// sessions and a busy session.
pub fn inbox(agent: Agent, options: &Options) -> Result<bool> {
    let (work, inbox, path) = start_with(agent, INBOX_BIN)?;
    let setup = Setup {
        work: work.path(),
        agent,
        inbox: &inbox,
        model: options.model,
        path: &path,
    };
    let mut check = InboxCheck::default();

    // Idle: a message waits for session a; with --all, session b, in its own project with its own
    // inbox, runs at the same time.
    let sessions = Sessions {
        options,
        prepare: |name: &str, kind, stub: Option<&Stub>| setup.run(name, kind, stub),
        add: add_message,
    };
    let idle = sessions.idle(Kind::Idle.script(), |run, stub| {
        run_session(run, stub, |_| {})
    })?;
    println!("{}", check.idle(&idle.a, &idle.token, idle.state));
    if let Some(session_b) = idle.b {
        println!("{}", check.other(&session_b?, &idle.token));
    }

    // Busy: the message arrives while the agent runs a shell command.
    if options.all {
        let c = sessions.busy(Kind::Busy.script(), |_| {})?;
        println!(
            "{}",
            check.busy(&c.session, &c.token, c.added_at_ms, c.state)
        );
    }

    let verdict = check.verdict(agent, options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// A hook's live check on `agent`: a session with a message waiting at its start; with `--all`,
/// also the other of two sessions and, but for `UserPromptSubmit`, a busy session.
pub fn hook(agent: Agent, hook: Hook, options: &Options) -> Result<bool> {
    let (work, handler, path) = start_with(agent, HOOK_BIN)?;
    let live_check = std::env::current_exe()?;
    let setup = hook::Setup {
        work: work.path(),
        agent,
        hook,
        handler: &handler,
        live_check: &live_check,
        model: options.model,
        path: &path,
    };
    let mut check = HookCheck::new(agent, hook);

    // A message waits for session a; with --all, session b, with its own empty source, runs at the
    // same time.
    let timed = |run: &Run, stub: Option<&Stub>| -> Result<(Session, HookRuns)> {
        let mut runs = HookRuns::new(hook);
        let session = run_session(run, stub, |event| runs.on_event(event, now_ms()))?;
        read_hook_runs(run, &mut runs);
        Ok((session, runs))
    };
    let sessions = Sessions {
        options,
        prepare: |name: &str, kind, stub: Option<&Stub>| setup.run(name, kind, stub),
        add: channel::add_message,
    };
    let idle = sessions.idle(hook.script(Kind::Idle), timed)?;
    let (session_a, runs_a) = &idle.a;
    println!(
        "{}",
        check.waiting(session_a, runs_a, &idle.token, idle.state)
    );
    if let Some(session_b) = idle.b {
        let (session_b, runs_b) = session_b?;
        println!("{}", check.other(&session_b, &runs_b, &idle.token));
    }

    // Busy: the message arrives while the agent runs a shell command.
    if options.all && hook.checks_busy() {
        let mut runs = HookRuns::new(hook);
        let c = sessions.busy(hook.script(Kind::Busy), |event| {
            runs.on_event(event, now_ms())
        })?;
        read_hook_runs(&c.run, &mut runs);
        println!(
            "{}",
            check.busy(&c.session, &runs, &c.token, c.added_at_ms, c.state)
        );
    }

    let verdict = check.verdict(options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Adds to `runs` the runs `live-check timed-hook` recorded in `run`'s session: Codex shows no hook
/// events. Claude Code's sessions record none.
fn read_hook_runs(run: &Run, runs: &mut HookRuns) {
    if let Ok(text) = std::fs::read_to_string(hook_runs_file(run)) {
        runs.read_log(&text);
    }
}

/// Reads `stdout` line by line in a thread of its own: each line with when it came, in
/// milliseconds since the Unix epoch. The receiver disconnects at the end of `stdout`.
pub fn read_lines(stdout: ChildStdout) -> Receiver<(String, u64)> {
    let (lines, received) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
            if lines.send((line, now_ms())).is_err() {
                break;
            }
        }
    });
    received
}

/// Runs `run`'s session to its end, calling `on_event` with each event; `stub` is its model stub.
pub(crate) fn run_session(
    run: &Run,
    stub: Option<&Stub>,
    mut on_event: impl FnMut(&Event),
) -> Result<Session> {
    let mut command = run.agent.command();
    if let Some(env) = &run.env {
        command
            .env_clear()
            .envs(env.iter().map(|(name, value)| (name, value)));
    }
    let mut child = command
        .args(&run.args)
        .current_dir(&run.project)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(&run.stderr)?)
        .spawn()?;
    let mut session = Session::new(now_ms());
    let received = read_lines(child.stdout.take().expect("stdout is piped"));
    let mut log = std::fs::File::create(&run.log)?;
    let deadline = Instant::now() + TIMEOUT;
    let mut timed_out = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = match received.recv_timeout(left) {
            Ok((line, _)) => line,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                child.kill()?;
                timed_out = true;
                break;
            }
        };
        writeln!(log, "{line}")?;
        for event in run.agent.events(&line) {
            session.record(&event);
            on_event(&event);
        }
    }
    let status = child.wait()?;
    // A session on a stub that fails each request is meant to fail.
    let expected = stub.is_some_and(Stub::fails);
    if let Some(failure) = session_failure(timed_out, TIMEOUT, status, expected) {
        eprintln!(
            "live-check: session {} {failure}; see {}",
            run.name,
            run.stderr.display()
        );
    }
    let path = run.log.with_file_name("stub.json");
    session.stub = stub.map(|stub| stub.saved_log(&path)).transpose()?;
    Ok(session)
}
