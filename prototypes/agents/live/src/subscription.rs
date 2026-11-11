//! Runs the live checks of the subscription limits: `agent-limits` on Claude Code's `/usage` and on
//! the Codex app-server, and Claude Code's status line in a pseudo-terminal, with one turn of the
//! real model. Everything that starts no process is in the library's `limits` module.

use std::fs::{self, File};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use agent_live::channel::{Files, extra_environment};
use agent_live::limits::{
    self, LIMITS_BIN, LimitsCheck, Read, codex_config_file, shown, status_line_limits,
    status_line_log, status_line_session, status_line_usage, usage_text,
};
use agent_live::{Agent, Options, binary};
use agent_source::now_ms;

use crate::Result;
use crate::pty::{Env, Prepared, turn_ended};
use crate::run::{finish, start};

/// The longest wait for the status line session's turn.
const TURN: Duration = Duration::from_secs(120);

/// How long after the turn the status line may take to get the limits.
const LATE: Duration = Duration::from_secs(10);

/// Now, in seconds since the Unix epoch.
fn now_s() -> i64 {
    i64::try_from(now_ms() / 1000).unwrap_or(i64::MAX)
}

/// The live check of Claude Code's `/usage`: `agent-limits claude` reads each limit it shows.
pub fn usage(options: &Options) -> Result<bool> {
    let work = start(Agent::Claude)?;
    let agent_limits = binary(&std::env::current_exe()?, LIMITS_BIN)?;
    let mut check = LimitsCheck::default();
    let (read, output) = read_claude(&work.path().join("usage"), &agent_limits)?;
    println!(
        "{}",
        check.usage(&read, &shown(&usage_text(&output)), now_s())
    );
    let verdict = check.verdict("/usage", Agent::Claude);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The live check of Claude Code's status line: after one short turn, the limits of its input are
/// those `/usage` then shows.
pub fn status_line(options: &Options) -> Result<bool> {
    let work = start(Agent::Claude)?;
    let agent_limits = binary(&std::env::current_exe()?, LIMITS_BIN)?;
    let files = Files::new(&work.path().join("status-line"));
    let log = status_line_log(&files);
    let args = status_line_session(&files, &agent_limits, options.model)?;
    let env = Env::Add(extra_environment(true));
    let prepared = Prepared::new("status-line", Agent::Claude, false, files, args, env, None)?;
    let mut live = prepared.start()?;
    let read_log = || fs::read_to_string(&log).unwrap_or_default();
    if live.wait(TURN, turn_ended)? {
        live.wait(LATE, |_| !status_line_limits(&read_log()).1.is_empty())?;
    }
    let mut session = live.end()?;
    // Claude Code's terminal UI shows no tokens; its status line input does.
    session.usage = status_line_usage(&read_log());
    let (inputs, status) = status_line_limits(&read_log());
    let (usage, _) = read_claude(&work.path().join("usage"), &agent_limits)?;
    let mut check = LimitsCheck::default();
    println!(
        "{}",
        check.status_line(&session.summary(), inputs, &status, &usage.limits, now_s())
    );
    let verdict = check.verdict("the status line", Agent::Claude);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// The live check of the Codex app-server: `agent-limits codex` reads limits, each with its reset.
pub fn codex(options: &Options) -> Result<bool> {
    let work = start(Agent::Codex)?;
    let agent_limits = binary(&std::env::current_exe()?, LIMITS_BIN)?;
    let config = codex_config_file(
        std::env::var_os("CODEX_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
    .and_then(|file| fs::read_to_string(file).ok())
    .unwrap_or_default();
    let dir = work.path().join("app-server");
    let args = limits::codex(&dir.join("exchange.jsonl"), &config);
    let read = run(&dir, Agent::Codex, &agent_limits, &args)?;
    let mut check = LimitsCheck::default();
    println!("{}", check.codex(&read, now_s()));
    let verdict = check.verdict("the app-server", Agent::Codex);
    Ok(finish(work, options, &verdict, check.pass()))
}

/// Runs `agent-limits claude` in `dir`; returns what it printed and the CLI's output.
fn read_claude(dir: &Path, agent_limits: &Path) -> Result<(Read, String)> {
    let log = dir.join("usage.jsonl");
    let read = run(dir, Agent::Claude, agent_limits, &limits::claude(&log))?;
    Ok((read, fs::read_to_string(&log).unwrap_or_default()))
}

/// Runs `agent_limits <args>` in the CLI's environment of `agent`, in an empty project in `dir`;
/// returns what it printed. Its stderr goes to `dir`.
fn run(dir: &Path, agent: Agent, agent_limits: &Path, args: &[String]) -> Result<Read> {
    let project = dir.join("project");
    fs::create_dir_all(&project)?;
    let stderr = dir.join("agent-limits.stderr");
    let output = agent
        .command_for(agent_limits)
        .args(args)
        .current_dir(&project)
        .stdin(Stdio::null())
        .stderr(File::create(&stderr)?)
        .output()?;
    if !output.status.success() {
        eprintln!(
            "live-check: agent-limits {}; see {}",
            output.status,
            stderr.display()
        );
    }
    Ok(Read::parse(&String::from_utf8_lossy(&output.stdout)))
}
