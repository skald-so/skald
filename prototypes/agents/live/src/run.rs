//! Starts the installed CLIs and reads their output; everything else is in the library.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use agent_live::{
    Agent, BusyMessage, Event, InboxCheck, Options, Run, Session, add_message, busy_prompt,
    idle_prompt, inbox_server, session_failure, token,
};
use agent_source::Source;

/// The longest a session may take.
const TIMEOUT: Duration = Duration::from_secs(300);

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The `inbox` path's live check: an idle session that is one of two, then a busy session.
pub fn inbox(agent: Agent, options: &Options) -> Result<bool> {
    let version = Command::new(agent.cli()).arg("--version").output()?;
    println!("{}", agent.version(&version.stdout));
    let work = tempfile::Builder::new().prefix("skald-live-").tempdir()?;
    let inbox = inbox_server(&std::env::current_exe()?)?;
    let model = options.model;
    let mut check = InboxCheck::default();

    // Idle: two sessions, each in its project with its own inbox; a message waits for the first.
    let a = Run::new(work.path(), "a", agent, &inbox, &idle_prompt(), model)?;
    let b = Run::new(work.path(), "b", agent, &inbox, &idle_prompt(), model)?;
    let source_a = Source::open(&a.source)?;
    let token_a = token();
    let message = add_message(&source_a, &token_a)?;
    let (session_a, session_b) = thread::scope(|scope| {
        let session_b = scope.spawn(|| run_session(&b, |_| {}));
        (
            run_session(&a, |_| {}),
            session_b.join().expect("session b ran"),
        )
    });
    let (session_a, session_b) = (session_a?, session_b?);
    let state = source_a.state(&message.msg_id)?;
    println!("{}", check.idle(&session_a, &token_a, state));
    println!("{}", check.other(&session_b, &token_a));

    // Busy: the message arrives while the agent runs a shell command.
    let c = Run::new(work.path(), "c", agent, &inbox, &busy_prompt(), model)?;
    let source_c = Source::open(&c.source)?;
    let token_c = token();
    let mut busy = BusyMessage::default();
    let session_c = run_session(&c, |event| {
        busy.on_event(event, || add_message(&source_c, &token_c));
    })?;
    let message = busy.added().transpose()?;
    let state = match &message {
        Some(message) => source_c.state(&message.msg_id)?,
        None => None,
    };
    let added_at_ms = message.map(|message| message.added_at_ms);
    println!("{}", check.busy(&session_c, &token_c, added_at_ms, state));

    println!("{}", check.verdict(agent));
    if options.keeps_logs(check.pass()) {
        println!("logs: {}", work.keep().display());
    }
    Ok(check.pass())
}

/// Runs `run`'s session to its end, calling `on_event` with each event.
fn run_session(run: &Run, mut on_event: impl FnMut(&Event)) -> Result<Session> {
    let mut child = Command::new(run.agent.cli())
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
        let Some(event) = run.agent.event(&line) else {
            continue;
        };
        session.record(&event);
        on_event(&event);
    }
    let status = child.wait()?;
    if let Some(failure) = session_failure(timed_out, TIMEOUT, status) {
        eprintln!(
            "live-check: session {} {failure}; see {}",
            run.name,
            run.stderr.display()
        );
    }
    Ok(session)
}
