//! The GUI live checks' commands: `gui` prepares a check for the developer or reviewer who drives
//! the surface, `gui-add` adds the busy case's message, `gui-queue` queues it into the session's
//! thread, `gui-result` takes the token they read. Everything else is in the library's `gui`
//! module.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use agent_app_server::{Client, Stdio};
use agent_live::gui::{self, Binaries, GuiPath, Installed, Surface};
use agent_live::hook::HOOK_BIN;
use agent_live::limits::{mcp_servers, quiet_app_server};
use agent_live::wake::Wake;
use agent_live::{Agent, INBOX_BIN, binary, channel::CHANNEL_BIN, queue, seconds, token};
use agent_source::Source;
use serde_json::json;

use crate::Result;
use crate::subscription::user_codex_config;

/// Prepares the GUI check of `path` on `surface` in a new temporary directory, which stays: prints
/// the surface's version, the project to open, the VS Code profile it needs if any, the prompt to
/// type and the commands to run next. It never prints the token. In the `busy` case the message
/// waits for `gui-add`; for `codex queue`, for `gui-queue`; for the paths that wake a session, for
/// `gui-add` or, for the socket, `gui-socket`. A `bypass` session skips permission prompts.
pub fn prepare(path: GuiPath, surface: Surface, busy: bool, bypass: bool) -> Result<bool> {
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
    let plugin = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../mod"))?;
    let profile = gui::vscode_profile(&work, surface, path, &plugin, bypass)?;
    println!("{version}");
    println!("project: {}", prepared.project.display());
    if let Some(profile) = profile {
        println!("VS Code profile: {}", profile.display());
    }
    if bypass {
        println!("mode: bypass permissions, before the first prompt");
    }
    println!("prompt: {}", prepared.prompt);
    let when = if busy {
        "once the shell command runs"
    } else {
        "once the first turn ended"
    };
    if path.needs_thread() {
        println!("{when}: live-check gui-queue {} [--cli]", work.display());
    } else if path == GuiPath::Wake(Wake::Socket) {
        println!(
            "{when}: live-check gui-socket {} [--no-auth] [--listen <seconds>]",
            work.display()
        );
    } else if busy || path.wakes() {
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
    if let Some(confirmations) = &outcome.confirmations {
        println!("{confirmations}");
    }
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
    let (mut client, thread) = session_thread(&codex, &pending)?;
    let thread = thread.ok_or_else(|| {
        format!(
            "no thread in {} started with the check's prompt: send it first",
            pending.project.display()
        )
    })?;
    println!("thread: {thread}");
    gui::set_thread(work, &thread)?;
    let message = gui::add(work)?;
    println!("{}", message.msg_id);
    let source = Source::open(work.join("source"))?;
    let handed = agent_queue::hand_over(&source, &thread, Agent::Codex.command_for(&codex))?;
    println!("handed over: {}", handed.len());
    if !handed.is_empty()
        && let Some(after) = queue::taken(&mut client, &thread, &source, &message.msg_id, QUEUED)?
    {
        println!(
            "taken from the queue: {}",
            seconds(after.as_millis() as u64)
        );
    }
    Ok(!handed.is_empty())
}

/// Hands the message of the socket check prepared in `work` to the session through its inbox
/// socket: adds it the first time, and sends it again once it was handed over, such as after the
/// session held it; with the session's token unless `auth` is false. With `listen`, from a reply
/// address it keeps for that long, printing each receipt. Prints the message's `msg_id`, never its
/// token.
pub fn socket(work: &Path, auth: bool, listen: Option<Duration>) -> Result<bool> {
    let source = Source::open(work.join("source"))?;
    let registration = agent_socket::registered(&source)?;
    match gui::added(work)? {
        None => println!("{}", gui::add(work)?.msg_id),
        Some((_, msg_id)) => {
            if matches!(
                source.state(&msg_id)?,
                Some(agent_source::State::HandedOver { .. })
            ) {
                source.requeue(std::slice::from_ref(&msg_id))?;
            }
            println!("{msg_id}");
        }
    }
    let Some(listen) = listen else {
        let handed = agent_socket::hand_over(&source, auth, agent_socket::FROM)?;
        println!("handed over: {}", handed.len());
        return Ok(!handed.is_empty());
    };
    let receipts =
        agent_socket::Receipts::bind(Path::new(&registration.socket.unwrap_or_default()))?;
    let handed = agent_socket::hand_over(&source, auth, &receipts.address())?;
    println!("handed over: {}", handed.len());
    let started = Instant::now();
    receipts.collect(&source, started + listen, |receipt| {
        println!(
            "{} after {}",
            receipt.status,
            seconds(started.elapsed().as_millis() as u64)
        );
    })?;
    Ok(!handed.is_empty())
}

/// How long `gui-queue` waits for the queued message to leave the thread's queue.
const QUEUED: Duration = Duration::from_secs(120);

/// A client of `codex app-server` on the user's config home, which starts no thread and no MCP
/// server, and the thread of the session that `pending` waits for, from its `thread/list`.
fn session_thread(
    codex: &Path,
    pending: &gui::Pending,
) -> Result<(Client<Stdio, io::Sink>, Option<String>)> {
    let config = user_codex_config();
    let mut command = Agent::Codex.command_for(codex);
    // Its stderr would show the user's config.
    command
        .arg("app-server")
        .args(quiet_app_server(&config))
        .stderr(std::process::Stdio::null());
    let stop = Arc::new(AtomicBool::new(false));
    let mut client = Client::new(Stdio::spawn(command)?, io::sink(), stop);
    // The queue's methods are experimental.
    client.initialize_experimental()?;
    // Codex keeps the directory as it got it or with its symbolic links resolved.
    let cwd = [pending.project.clone(), fs::canonicalize(&pending.project)?];
    let list = client.call("thread/list", json!({ "cwd": cwd }))?;
    let thread = gui::session_thread(&list, &pending.prompt);
    Ok((client, thread))
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
