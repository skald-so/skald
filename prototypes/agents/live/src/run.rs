//! Starts the installed CLIs and reads their output; everything else is in the library.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use agent_live::hook::{self, HOOK_BIN, Hook, HookCheck, HookRuns, hook_runs_file};
use agent_live::{
    Agent, BusyMessage, Event, INBOX_BIN, InboxCheck, Kind, Options, Run, Session, Setup, Stub,
    add_message, binary, channel, session_failure, token,
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

/// When the busy session's message was added and where it is now; neither if the agent ran no
/// shell command.
fn busy_outcome(
    busy: BusyMessage<io::Result<Message>>,
    source: &Source,
) -> Result<(Option<u64>, Option<State>)> {
    let Some(message) = busy.added().transpose()? else {
        return Ok((None, None));
    };
    Ok((Some(message.added_at_ms), source.state(&message.msg_id)?))
}

/// The `inbox` path's live check: an idle session; with `--all`, also the other of two idle
/// sessions and a busy session.
pub fn inbox(agent: Agent, options: &Options) -> Result<bool> {
    let work = start(agent)?;
    let inbox = binary(&std::env::current_exe()?, INBOX_BIN)?;
    let path = std::env::var("PATH").unwrap_or_default();
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
    let stub_a = options.stub(Kind::Idle.script())?;
    let a = setup.run("a", Kind::Idle, stub_a.as_ref())?;
    let source_a = Source::open(&a.source)?;
    let token_a = token();
    let message = add_message(&source_a, &token_a)?;
    let b = if options.all {
        let stub = options.stub(Kind::Idle.script())?;
        Some((setup.run("b", Kind::Idle, stub.as_ref())?, stub))
    } else {
        None
    };
    let (session_a, session_b) = side_by_side(
        || run_session(&a, stub_a.as_ref(), |_| {}),
        b.as_ref()
            .map(|(b, stub)| || run_session(b, stub.as_ref(), |_| {})),
    );
    let session_a = session_a?;
    let state = source_a.state(&message.msg_id)?;
    println!("{}", check.idle(&session_a, &token_a, state));
    if let Some(session_b) = session_b {
        println!("{}", check.other(&session_b?, &token_a));
    }

    // Busy: the message arrives while the agent runs a shell command.
    if options.all {
        let stub = options.stub(Kind::Busy.script())?;
        let c = setup.run("c", Kind::Busy, stub.as_ref())?;
        let source_c = Source::open(&c.source)?;
        let token_c = token();
        let mut busy = BusyMessage::default();
        let session_c = run_session(&c, stub.as_ref(), |event| {
            busy.on_event(event, || add_message(&source_c, &token_c));
        })?;
        let (added_at_ms, state) = busy_outcome(busy, &source_c)?;
        println!("{}", check.busy(&session_c, &token_c, added_at_ms, state));
    }

    let verdict = check.verdict(agent, options);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// A hook's live check on `agent`: a session with a message waiting at its start; with `--all`,
/// also the other of two sessions and, but for `UserPromptSubmit`, a busy session.
pub fn hook(agent: Agent, hook: Hook, options: &Options) -> Result<bool> {
    let work = start(agent)?;
    let live_check = std::env::current_exe()?;
    let handler = binary(&live_check, HOOK_BIN)?;
    let path = std::env::var("PATH").unwrap_or_default();
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
    let stub_a = options.stub(hook.script(Kind::Idle))?;
    let a = setup.run("a", Kind::Idle, stub_a.as_ref())?;
    let source_a = Source::open(&a.source)?;
    let token_a = token();
    let message = channel::add_message(&source_a, &token_a)?;
    let b = if options.all {
        let stub = options.stub(hook.script(Kind::Idle))?;
        Some((setup.run("b", Kind::Idle, stub.as_ref())?, stub))
    } else {
        None
    };
    let timed = |run: &Run, stub: Option<&Stub>| -> Result<(Session, HookRuns)> {
        let mut runs = HookRuns::new(hook);
        let session = run_session(run, stub, |event| runs.on_event(event, now_ms()))?;
        read_hook_runs(run, &mut runs);
        Ok((session, runs))
    };
    let (session_a, session_b) = side_by_side(
        || timed(&a, stub_a.as_ref()),
        b.as_ref().map(|(b, stub)| || timed(b, stub.as_ref())),
    );
    let (session_a, runs_a) = session_a?;
    let state = source_a.state(&message.msg_id)?;
    println!("{}", check.waiting(&session_a, &runs_a, &token_a, state));
    if let Some(session_b) = session_b {
        let (session_b, runs_b) = session_b?;
        println!("{}", check.other(&session_b, &runs_b, &token_a));
    }

    // Busy: the message arrives while the agent runs a shell command.
    if options.all && hook.checks_busy() {
        let stub = options.stub(hook.script(Kind::Busy))?;
        let c = setup.run("c", Kind::Busy, stub.as_ref())?;
        let source_c = Source::open(&c.source)?;
        let token_c = token();
        let mut busy = BusyMessage::default();
        let mut runs = HookRuns::new(hook);
        let session_c = run_session(&c, stub.as_ref(), |event| {
            runs.on_event(event, now_ms());
            busy.on_event(event, || channel::add_message(&source_c, &token_c));
        })?;
        read_hook_runs(&c, &mut runs);
        let (added_at_ms, state) = busy_outcome(busy, &source_c)?;
        println!(
            "{}",
            check.busy(&session_c, &runs, &token_c, added_at_ms, state)
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
fn run_session(
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
    if let Some(failure) = session_failure(timed_out, TIMEOUT, status) {
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
