//! The code that runs the installed CLIs; the verdicts are in the library.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use agent_live::{Agent, Event, Options, Session, handover_delay, report, verdict};
use agent_source::Source;

/// The longest a session may take.
const TIMEOUT: Duration = Duration::from_secs(300);
/// The sender label of the checks' messages.
const SENDER: &str = "live-check of skald";

const IDLE_PROMPT: &str = "Call the inbox tool of the skald MCP server, then reply with only the \
    check token from the message, or NONE if there is none.";
const BUSY_PROMPT: &str = "Run the shell command `sleep 20`. Then call the inbox tool of the skald \
    MCP server and reply with only the check token from the message, or NONE if there is none.";

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// The `inbox` path's live check: an idle session that is one of two, then a busy session.
pub fn inbox(agent: Agent, options: &Options) -> Result<bool> {
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
    let model = options.model;

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
    let idle = session_a.received(&token_a, state);
    println!(
        "{}",
        report(
            "idle session, one of two",
            idle,
            &format!(
                "{}; handed over {} after the session started",
                session_a.summary(),
                handover_delay(state, session_a.started_ms)
            ),
        )
    );
    let other = session_b.missed(&token_a);
    println!(
        "{}",
        report(
            "the other idle session",
            other,
            &format!("{}; the message did not reach it", session_b.summary()),
        )
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
    let (state, details) = match added {
        Some(message) => {
            let message = message?;
            let state = source_c.state(&message.msg_id)?;
            let delay = handover_delay(state, message.added_at_ms);
            (
                state,
                format!("added while the shell command ran, handed over {delay} later"),
            )
        }
        None => (None, "the agent ran no shell command".to_owned()),
    };
    let busy = session_c.received(&token_c, state);
    println!(
        "{}",
        report(
            "busy session",
            busy,
            &format!("{}; {details}", session_c.summary())
        )
    );

    let pass = idle && other && busy;
    println!("inbox on {}: {}", agent.cli(), verdict(pass));
    if options.keep || !pass {
        println!("logs: {}", work.keep().display());
    }
    Ok(pass)
}

/// A random check token.
fn token() -> String {
    format!("tok-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
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
            session.record(&event);
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
