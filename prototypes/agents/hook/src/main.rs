//! `agent-hook [--codex] <source-dir>`: a command hook of Claude Code, or with `--codex` of Codex,
//! for `PostToolUse` and `Stop`, on Codex `UserPromptSubmit`, and `SessionStart`, which registers
//! the session. Reads the hook input on stdin and prints the waiting messages of the source as the
//! hook's output. It fails with exit code 1, never 2: on `Stop`, exit code 2 would keep the turn
//! going with stderr as the agent's next instruction; on Codex's `PostToolUse`, it would replace the
//! tool result.
//!
//! Its other modes serve paths that are not settings hooks of these events, or confirm only:
//! - `--confirm [--codex] <source-dir>`: a settings hook that only records the `transcript` signal
//!   of the handed-over messages the hook input's session file shows, and hands nothing over.
//! - `--wait <source-dir> <seconds>`: Claude Code's `asyncRewake` hook. Waits up to `<seconds>` for
//!   a message, prints the waiting messages to stderr and exits 2, which wakes the session; exits 0
//!   at the end of the wait, or at once while another `--wait` waits on the source.
//! - `--take <source-dir>`, `--count <source-dir>`, `--release <source-dir> <msg_id>...`: the
//!   Claude Code mod's helper. `--take` prints the waiting messages as `{"ids", "text"}` and marks
//!   them handed over, nothing if none wait; `--count` prints how many wait; `--release` puts
//!   handed-over messages back, when the session dropped the prompt.

use std::fs::File;
use std::io::{self, Read, Write};

use std::process::ExitCode;
use std::time::{Duration, Instant};

use agent_hook::{Agent, Messaging};
use agent_inbox::render;
use agent_source::Source;

const USAGE: &str = "usage: agent-hook [--codex] <source-dir>\n       \
    agent-hook --confirm [--codex] <source-dir>\n       \
    agent-hook --wait <source-dir> <seconds>\n       \
    agent-hook --take <source-dir>\n       \
    agent-hook --count <source-dir>\n       \
    agent-hook --release <source-dir> <msg_id>...";

fn main() -> ExitCode {
    let owned: Vec<String> = std::env::args().skip(1).collect();
    let ran = match owned.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["--wait", dir, seconds] => match seconds.parse() {
            Ok(seconds) => with_source(dir, |source| wait(source, Duration::from_secs(seconds))),
            Err(_) => return usage(),
        },
        ["--take", dir] => with_source(dir, |source| {
            let mut out = io::stdout().lock();
            source.hand_over(|messages| {
                writeln!(out, "{}", agent_hook::taken(messages)).and_then(|()| out.flush())
            })?;
            Ok(ExitCode::SUCCESS)
        }),
        ["--count", dir] => with_source(dir, |source| {
            println!("{}", source.waiting_count()?);
            Ok(ExitCode::SUCCESS)
        }),
        ["--release", dir, ref ids @ ..] if !ids.is_empty() => with_source(dir, |source| {
            let ids: Vec<String> = ids.iter().map(|id| (*id).to_owned()).collect();
            source.requeue(&ids)?;
            Ok(ExitCode::SUCCESS)
        }),
        ["--confirm", dir] if !dir.starts_with("--") => confirm(Agent::Claude, dir),
        ["--confirm", "--codex", dir] => confirm(Agent::Codex, dir),
        ["--codex", dir] => hook(Agent::Codex, dir),
        [dir] if !dir.starts_with("--") => hook(Agent::Claude, dir),
        _ => return usage(),
    };
    ran.unwrap_or_else(|error| {
        eprintln!("agent-hook: {error}");
        ExitCode::FAILURE
    })
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::FAILURE
}

/// Runs `run` on the source in `dir`.
fn with_source(
    dir: &str,
    run: impl FnOnce(&Source) -> io::Result<ExitCode>,
) -> io::Result<ExitCode> {
    let source = Source::open(dir)
        .map_err(|error| io::Error::other(format!("cannot open the source {dir}: {error}")))?;
    run(&source)
}

/// Runs `run` on the source in `dir` and the hook input on stdin.
fn with_input(
    dir: &str,
    run: impl FnOnce(&Source, &[u8]) -> io::Result<()>,
) -> io::Result<ExitCode> {
    with_source(dir, |source| {
        let mut input = Vec::new();
        io::stdin().read_to_end(&mut input)?;
        run(source, &input)?;
        Ok(ExitCode::SUCCESS)
    })
}

/// The settings hook of `agent` on the source in `dir`.
fn hook(agent: Agent, dir: &str) -> io::Result<ExitCode> {
    with_input(dir, |source, input| {
        let out = &mut io::stdout().lock();
        agent_hook::handle(agent, input, source, out, &Messaging::from_env())
    })
}

/// The confirm-only hook of `agent` on the source in `dir`.
fn confirm(agent: Agent, dir: &str) -> io::Result<ExitCode> {
    with_input(dir, |source, input| {
        let input: serde_json::Value = serde_json::from_slice(input)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some(transcript) = input["transcript_path"].as_str() {
            agent_hook::confirm(agent, std::path::Path::new(transcript), source)?;
        }
        Ok(())
    })
}

/// Waits up to `limit` for a message in `source`, as one waiter per source: hands the waiting
/// messages over on stderr and exits 2.
fn wait(source: &Source, limit: Duration) -> io::Result<ExitCode> {
    // Claude Code gives the hook its input; the wait needs none.
    let _ = io::copy(&mut io::stdin(), &mut io::sink());
    let lock = File::create(source.dir().join("rewake.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(ExitCode::SUCCESS);
    }
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        let mut err = io::stderr().lock();
        let handed = source
            .hand_over(|messages| write!(err, "{}", render(messages)).and_then(|()| err.flush()))?;
        if !handed.is_empty() {
            return Ok(ExitCode::from(2));
        }
        drop(err);
        std::thread::sleep(Duration::from_millis(200));
    }
    Ok(ExitCode::SUCCESS)
}
