//! Live checks of the delivery paths against the installed CLIs. `script/test` does not run them.
//!
//! ```text
//! live-check add <source-dir> <sender> <text>
//! live-check inbox <claude|codex> [--model <model>] [--keep]
//! ```

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use agent_live::{Agent, Event};
use agent_source::{Source, State};

/// The longest a session may take.
const TIMEOUT: Duration = Duration::from_secs(300);
/// The sender label of the checks' messages.
const SENDER: &str = "live-check of skald";

const IDLE_PROMPT: &str = "Call the inbox tool of the skald MCP server, then reply with only the \
    check token from the message, or NONE if there is none.";
const BUSY_PROMPT: &str = "Run the shell command `sleep 20`. Then call the inbox tool of the skald \
    MCP server and reply with only the check token from the message, or NONE if there is none.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["add", dir, sender, text] => match Source::open(dir).and_then(|s| s.add(sender, text)) {
            Ok(message) => {
                println!("{}", message.msg_id);
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("live-check: {error}");
                ExitCode::FAILURE
            }
        },
        ["inbox", agent, options @ ..] => {
            let (Some(agent), Some((model, keep))) = (Agent::parse(agent), options_of(options))
            else {
                return usage();
            };
            match inbox(agent, model, keep) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::FAILURE,
                Err(error) => {
                    eprintln!("live-check: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: live-check add <source-dir> <sender> <text>\n       \
         live-check inbox <claude|codex> [--model <model>] [--keep]"
    );
    ExitCode::from(2)
}

/// `--model <model>` and `--keep`.
fn options_of<'a>(mut options: &[&'a str]) -> Option<(Option<&'a str>, bool)> {
    let (mut model, mut keep) = (None, false);
    loop {
        match options {
            [] => return Some((model, keep)),
            ["--model", value, rest @ ..] => (model, options) = (Some(*value), rest),
            ["--keep", rest @ ..] => (keep, options) = (true, rest),
            _ => return None,
        }
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The `inbox` path's live check: an idle session that is one of two, then a busy session.
fn inbox(agent: Agent, model: Option<&str>, keep: bool) -> Result<bool> {
    let version = Command::new(agent.cli()).arg("--version").output()?;
    println!(
        "{}: {}",
        agent.cli(),
        String::from_utf8_lossy(&version.stdout).trim()
    );
    let work = tempfile::Builder::new().prefix("skald-live-").tempdir()?;
    let inbox = std::env::current_exe()?.with_file_name("agent-inbox");
    if !inbox.exists() {
        return Err(format!("{} is missing: build the workspace", inbox.display()).into());
    }

    // Idle: two sessions, each in its project with its own inbox; a message waits for the first.
    let a = Run::new(work.path(), "a", agent, &inbox, IDLE_PROMPT, model)?;
    let b = Run::new(work.path(), "b", agent, &inbox, IDLE_PROMPT, model)?;
    let source_a = Source::open(&a.source)?;
    let token_a = token();
    let message = source_a.add(SENDER, &format!("The check token is {token_a}."))?;
    let (session_a, session_b) = thread::scope(|scope| {
        let session_b = scope.spawn(|| b.run(|_| {}));
        (a.run(|_| {}), session_b.join().expect("session b ran"))
    });
    let (session_a, session_b) = (session_a?, session_b?);
    let state = source_a.state(&message.msg_id)?;
    let idle = session_a.passes(&token_a) && handed_over(state);
    report(
        "idle session, one of two",
        idle,
        &format!(
            "{}; handed over {} after the session started",
            session_a.summary(),
            after(state, session_a.started_ms)
        ),
    );
    let other = !session_b.reply.contains(&token_a) && session_b.inbox_called;
    report(
        "the other idle session",
        other,
        &format!("{}; the message did not reach it", session_b.summary()),
    );

    // Busy: the message arrives while the agent runs a shell command.
    let c = Run::new(work.path(), "c", agent, &inbox, BUSY_PROMPT, model)?;
    let source_c = Source::open(&c.source)?;
    let token_c = token();
    let mut added = None;
    let session_c = c.run(|event| {
        if *event == Event::ShellStarted && added.is_none() {
            added = Some(source_c.add(SENDER, &format!("The check token is {token_c}.")));
        }
    })?;
    let busy = match added {
        Some(message) => {
            let message = message?;
            let state = source_c.state(&message.msg_id)?;
            let pass = session_c.passes(&token_c) && handed_over(state);
            report(
                "busy session",
                pass,
                &format!(
                    "{}; added while the shell command ran, handed over {} later",
                    session_c.summary(),
                    after(state, message.added_at_ms)
                ),
            );
            pass
        }
        None => {
            report(
                "busy session",
                false,
                &format!("{}; the agent ran no shell command", session_c.summary()),
            );
            false
        }
    };

    let pass = idle && other && busy;
    println!(
        "inbox on {}: {}",
        agent.cli(),
        if pass { "pass" } else { "fail" }
    );
    if keep || !pass {
        println!("logs: {}", work.keep().display());
    }
    Ok(pass)
}

/// A random check token.
fn token() -> String {
    format!("tok-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

fn handed_over(state: Option<State>) -> bool {
    matches!(state, Some(State::HandedOver { .. }))
}

/// How long after `from_ms` the message was handed over.
fn after(state: Option<State>, from_ms: u64) -> String {
    match state {
        Some(State::HandedOver { at_ms }) => {
            format!("{:.1} s", at_ms.saturating_sub(from_ms) as f64 / 1000.0)
        }
        other => format!("never (state {other:?})"),
    }
}

fn report(check: &str, pass: bool, details: &str) {
    println!(
        "  {check}: {} — {details}",
        if pass { "pass" } else { "fail" }
    );
}

/// A session to run: its project, its inbox's source and its arguments.
struct Run {
    name: String,
    agent: Agent,
    project: PathBuf,
    source: PathBuf,
    args: Vec<String>,
    log: PathBuf,
}

/// What a session did.
struct Session {
    reply: String,
    sign_in: Option<String>,
    inbox_connected: Option<bool>,
    inbox_called: bool,
    started_ms: u64,
}

impl Run {
    fn new(
        work: &Path,
        name: &str,
        agent: Agent,
        inbox: &Path,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<Self> {
        let project = work.join(name).join("project");
        let source = work.join(name).join("source");
        std::fs::create_dir_all(&project)?;
        let args = agent.session(&project, inbox, &source, prompt, model)?;
        Ok(Self {
            name: name.to_owned(),
            agent,
            project,
            source,
            args,
            log: work.join(name).join("session.jsonl"),
        })
    }

    /// Runs the session to its end, calling `on_event` with each event.
    fn run(&self, mut on_event: impl FnMut(&Event)) -> Result<Session> {
        let stderr = self.log.with_extension("stderr");
        let mut child = Command::new(self.agent.cli())
            .args(&self.args)
            .current_dir(&self.project)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr)?)
            .spawn()?;
        let mut session = Session {
            reply: String::new(),
            sign_in: None,
            inbox_connected: None,
            inbox_called: false,
            started_ms: agent_source::now_ms(),
        };
        let stdout = child.stdout.take().expect("stdout is piped");
        let (lines, received) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(|line| line.ok()) {
                if lines.send(line).is_err() {
                    break;
                }
            }
        });
        let mut log = std::fs::File::create(&self.log)?;
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
            let Some(event) = self.agent.event(&line) else {
                continue;
            };
            match &event {
                Event::Started {
                    sign_in,
                    inbox_connected,
                } => {
                    session.sign_in = sign_in.clone();
                    session.inbox_connected = Some(*inbox_connected);
                }
                Event::InboxCalled => session.inbox_called = true,
                Event::Reply(text) => session.reply = text.clone(),
                Event::ShellStarted => {}
            }
            on_event(&event);
        }
        let status = child.wait()?;
        if timed_out || !status.success() {
            let how = if timed_out {
                format!("timed out after {} s", TIMEOUT.as_secs())
            } else {
                status.to_string()
            };
            eprintln!(
                "live-check: session {} {how}; see {}",
                self.name,
                stderr.display()
            );
        }
        Ok(session)
    }
}

impl Session {
    fn passes(&self, token: &str) -> bool {
        self.inbox_called && self.reply.contains(token)
    }

    fn summary(&self) -> String {
        let mut summary = format!("reply {:?}", self.reply.trim());
        if let Some(sign_in) = &self.sign_in {
            summary.push_str(&format!(", apiKeySource {sign_in}"));
        }
        if self.inbox_connected == Some(false) {
            summary.push_str(", the inbox server did not connect");
        }
        if !self.inbox_called {
            summary.push_str(", inbox not called");
        }
        summary
    }
}
