//! The GUI live checks' commands: `gui` prepares a check for the developer or reviewer who drives
//! the surface, `gui-add` adds the busy case's message, `gui-queue` queues it into the session's
//! thread, `gui-result` takes the token they read. Everything else is in the library's `gui`
//! module.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use agent_app_server::{Client, Stdio};
use agent_live::gui::{self, Binaries, GuiPath, Installed, Surface};
use agent_live::hook::HOOK_BIN;
use agent_live::limits::{mcp_servers, quiet_app_server};
use agent_live::{Agent, INBOX_BIN, binary, channel::CHANNEL_BIN, token};
use agent_source::Source;
use serde_json::json;

use crate::Result;
use crate::subscription::user_codex_config;

/// Prepares the GUI check of `path` on `surface` in a new temporary directory, which stays: prints
/// the surface's version, the project to open, the prompt to type and the commands to run next. It
/// never prints the token. In the `busy` case the message waits for `gui-add`; for `codex queue`,
/// for `gui-queue`.
pub fn prepare(path: GuiPath, surface: Surface, busy: bool) -> Result<bool> {
    let version = surface.version(&installed()?)?;
    let exe = std::env::current_exe()?;
    let binaries = Binaries {
        inbox: binary(&exe, INBOX_BIN)?,
        channel: binary(&exe, CHANNEL_BIN)?,
        hook: binary(&exe, HOOK_BIN)?,
        user_mcp_servers: mcp_servers(&user_codex_config()),
    };
    let work = tempfile::Builder::new()
        .prefix("skald-live-")
        .tempdir()?
        .keep();
    let prepared = gui::prepare(&work, surface, path, &binaries, &version, &token(), busy)?;
    println!("{version}");
    println!("project: {}", prepared.project.display());
    println!("prompt: {}", prepared.prompt);
    let when = if busy {
        "once the shell command runs"
    } else {
        "once the first turn ended"
    };
    if path.needs_thread() {
        println!("{when}: live-check gui-queue {} [--cli]", work.display());
    } else if busy {
        println!("{when}: live-check gui-add {}", work.display());
    }
    println!(
        "then: live-check gui-result {} <the token read from the reply>",
        work.display()
    );
    Ok(true)
}

/// Prints the surface's version and the outcome of the GUI check prepared in `work`, given the
/// token read from the reply.
pub fn result(work: &Path, token: &str) -> Result<bool> {
    let outcome = gui::result(work, token, &home()?)?;
    println!("{}", outcome.version);
    println!("{}", outcome.details);
    println!("{}", outcome.verdict);
    Ok(outcome.pass)
}

/// Adds the message of the GUI check prepared in `work` and prints its `msg_id`, not its token.
pub fn add(work: &Path) -> Result<bool> {
    println!("{}", gui::add(work)?.msg_id);
    Ok(true)
}

/// Adds the message of the `codex queue` check prepared in `work` and queues it into the session's
/// thread with the Codex CLI that the surface bundles or, with `cli`, the `codex` on `PATH`. The
/// thread is the newest in the check's project whose first prompt is the check's, as `thread/list`
/// of an app-server of that CLI on the user's config home gives it. Prints the thread, the
/// message's `msg_id` and how many messages it handed over; never the token.
pub fn queue(work: &Path, cli: bool) -> Result<bool> {
    let pending = gui::pending(work)?;
    let codex = if cli {
        PathBuf::from(Agent::Codex.cli())
    } else {
        pending.surface.codex(&installed()?)?
    };
    let thread = session_thread(&codex, &pending)?.ok_or_else(|| {
        format!(
            "no thread in {} started with the check's prompt: send it first",
            pending.project.display()
        )
    })?;
    println!("thread: {thread}");
    println!("{}", gui::add(work)?.msg_id);
    let source = Source::open(work.join("source"))?;
    let handed = agent_queue::hand_over(&source, &thread, Agent::Codex.command_for(&codex))?;
    println!("handed over: {}", handed.len());
    Ok(!handed.is_empty())
}

/// The thread of the session that `pending` waits for, from `thread/list` of `codex app-server` on
/// the user's config home, which starts no thread and no MCP server.
fn session_thread(codex: &Path, pending: &gui::Pending) -> Result<Option<String>> {
    let config = user_codex_config();
    let mut command = Agent::Codex.command_for(codex);
    // Its stderr would show the user's config.
    command
        .arg("app-server")
        .args(quiet_app_server(&config))
        .stderr(std::process::Stdio::null());
    let stop = Arc::new(AtomicBool::new(false));
    let mut client = Client::new(Stdio::spawn(command)?, io::sink(), stop);
    client.initialize()?;
    // Codex keeps the directory as it got it or with its symbolic links resolved.
    let cwd = [pending.project.clone(), fs::canonicalize(&pending.project)?];
    let list = client.call("thread/list", json!({ "cwd": cwd }))?;
    Ok(gui::session_thread(&list, &pending.prompt))
}

/// Where the surfaces are installed.
fn installed() -> Result<Installed> {
    Ok(Installed {
        applications: PathBuf::from("/Applications"),
        home: home()?,
    })
}

/// The user's home: VS Code's extensions and Claude Code's transcripts.
fn home() -> Result<PathBuf> {
    Ok(std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?)
}
