//! Starts the installed CLIs and reads their output; everything else is in the library.

use std::io::{BufRead, BufReader, Write};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use agent_live::{
    Agent, BusyMessage, Event, InboxCheck, Kind, Options, Run, Session, Setup, Stub, add_message,
    inbox_server, session_failure, token,
};
use agent_source::Source;

/// The longest a session may take.
const TIMEOUT: Duration = Duration::from_secs(300);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The `inbox` path's live check: an idle session; with `--all`, also the other of two idle
/// sessions and a busy session.
pub fn inbox(agent: Agent, options: &Options) -> Result<bool> {
    let version = agent.command().arg("--version").output()?;
    println!("{}", agent.version(&version.stdout));
    let work = tempfile::Builder::new().prefix("skald-live-").tempdir()?;
    let inbox = inbox_server(&std::env::current_exe()?)?;
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
    let stub_a = options.stub(Kind::Idle)?;
    let a = setup.run("a", Kind::Idle, stub_a.as_ref())?;
    let source_a = Source::open(&a.source)?;
    let token_a = token();
    let message = add_message(&source_a, &token_a)?;
    let b = if options.all {
        let stub = options.stub(Kind::Idle)?;
        Some((setup.run("b", Kind::Idle, stub.as_ref())?, stub))
    } else {
        None
    };
    let (session_a, session_b) = thread::scope(|scope| {
        let session_b = b
            .as_ref()
            .map(|(b, stub)| scope.spawn(|| run_session(b, stub.as_ref(), |_| {})));
        (
            run_session(&a, stub_a.as_ref(), |_| {}),
            session_b.map(|session| session.join().expect("session b ran")),
        )
    });
    let session_a = session_a?;
    let state = source_a.state(&message.msg_id)?;
    println!("{}", check.idle(&session_a, &token_a, state));
    if let Some(session_b) = session_b {
        println!("{}", check.other(&session_b?, &token_a));
    }

    // Busy: the message arrives while the agent runs a shell command.
    if options.all {
        let stub = options.stub(Kind::Busy)?;
        let c = setup.run("c", Kind::Busy, stub.as_ref())?;
        let source_c = Source::open(&c.source)?;
        let token_c = token();
        let mut busy = BusyMessage::default();
        let session_c = run_session(&c, stub.as_ref(), |event| {
            busy.on_event(event, || add_message(&source_c, &token_c));
        })?;
        let message = busy.added().transpose()?;
        let state = match &message {
            Some(message) => source_c.state(&message.msg_id)?,
            None => None,
        };
        let added_at_ms = message.map(|message| message.added_at_ms);
        println!("{}", check.busy(&session_c, &token_c, added_at_ms, state));
    }

    println!("{}", check.verdict(agent, options));
    if options.keeps_logs(check.pass()) {
        println!("logs: {}", work.keep().display());
    }
    Ok(check.pass())
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
    let mut session = Session::new(agent_source::now_ms());
    let stdout = child.stdout.take().expect("stdout is piped");
    let (lines, received) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut log = std::fs::File::create(&run.log)?;
    let deadline = Instant::now() + TIMEOUT;
    let mut timed_out = false;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = match received.recv_timeout(left) {
            Ok(line) => line,
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
    session.stub = stub.map(Stub::log);
    Ok(session)
}
