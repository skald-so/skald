//! Runs the live checks of the paths that tell that a Codex session waits on Codex's own permission
//! prompt: Codex's terminal UI in a pseudo-terminal, as a person answers the dialog there, and for
//! the app-server candidates a watcher of the app-server that answers nothing. Everything else is in
//! the library's `codex_permission` module.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use agent_app_server::{Received, Socket, Transport};
use agent_live::channel::{ENTER, Files, HOOKS, TERM};
use agent_live::codex_permission::{
    self, Answer, Candidate, DIALOG, NOTIFIED, Observed, PermissionCheck, WATCH, daemon_config,
    daemon_socket, watched_line,
};
use agent_live::hook::HOOK_RUNS;
use agent_live::{Agent, Options, Stub, queue};
use agent_source::now_ms;
use serde_json::{Value, json};

use crate::Result;
use crate::app::Server;
use crate::permission_run::{answer_dialog, shows};
use crate::pty::{Env, Live, Prepared, TICK};
use crate::run::{finish, start};

/// How long the session runs once the dialog is answered.
const AFTER: Duration = Duration::from_secs(10);

/// What the terminal UI shows once it takes a prompt: its footer's hint.
const COMPOSER: &str = "for shortcuts";

/// What the composer shows once the prompt is typed: its start.
const TYPED: &str = "Run the shell command";

/// The longest wait for the terminal UI to take a prompt, or for the app-server to listen.
const READY: Duration = Duration::from_secs(60);

/// The check of `candidate`: a session whose dialog the check answers yes; with `--all`, next to it
/// a session without a prompt, then a session whose dialog it answers no.
pub fn permission(candidate: Candidate, options: &Options) -> Result<bool> {
    let work = start(Agent::Codex)?;
    let check = Check {
        candidate,
        work: work.path(),
        live: &std::env::current_exe()?,
        path: &std::env::var("PATH").unwrap_or_default(),
        options,
    };
    let mut verdicts = PermissionCheck::new(candidate);
    let mut ran = check.pair("a", Answer::Yes, options.all.then_some("c"), &mut verdicts);
    if options.all
        && let Ok(lines) = &mut ran
    {
        match check.pair("b", Answer::No, None, &mut verdicts) {
            Ok(more) => lines.extend(more),
            Err(error) => ran = Err(error),
        }
    }
    // A check that broke off fails and keeps its logs.
    let lines = ran.unwrap_or_else(|error| vec![verdicts.broke(&error.to_string())]);
    for line in lines {
        println!("{line}");
    }
    let verdict = verdicts.verdict(options);
    Ok(finish(work, options, &verdict, verdicts.pass()))
}

/// What every session of the check shares.
struct Check<'a> {
    candidate: Candidate,
    work: &'a Path,
    /// `live-check`'s binary, which records the hook runs.
    live: &'a Path,
    path: &'a str,
    options: &'a Options<'a>,
}

impl Check<'_> {
    /// Runs the session `name`, whose dialog the check answers `answer`, and, if given, next to it
    /// the session `quiet`, without a prompt, on the same app-server and hooks file and in the same
    /// config home; returns their lines.
    fn pair(
        &self,
        name: &str,
        answer: Answer,
        quiet: Option<&str>,
        verdicts: &mut PermissionCheck,
    ) -> Result<Vec<String>> {
        let files = Files::new(&self.work.join(name));
        fs::create_dir_all(&files.project)?;
        let runs = files.dir.join(HOOK_RUNS);
        let watch = files.dir.join(WATCH);
        let stub = self.options.stub(codex_permission::script())?;
        let url = stub.as_ref().map(Stub::url);
        let home = files.dir.join("home");
        let codex = home.join(".codex");
        let quiet = quiet.map(|quiet| Files::new(&self.work.join(quiet)));
        // The app-server's config, or the background app-server's, trusts both sessions' projects.
        let mut projects = vec![files.project.as_path()];
        projects.extend(quiet.iter().map(|quiet| quiet.project.as_path()));
        let env = || {
            Env::of(
                &files,
                stub.as_ref(),
                self.path,
                queue::stub_environment,
                vec![("TERM".to_owned(), TERM.to_owned())],
            )
        };
        if self.candidate == Candidate::Daemon {
            fs::create_dir_all(&codex)?;
            let config = daemon_config(
                self.live,
                &runs,
                &projects,
                self.options.model,
                url.as_deref(),
            );
            fs::write(codex.join("config.toml"), config)?;
        }

        let (socket, server) = match self.candidate {
            Candidate::Remote => {
                let socket = self.work.join(format!("{name}.sock"));
                let args = codex_permission::app_server(
                    &socket,
                    &projects,
                    &runs,
                    self.live,
                    self.options.model,
                    url.as_deref(),
                );
                let mut server = Server::start(&args, &files, &env()?)?;
                server.wait_for(&socket)?;
                (Some(socket), Some(server))
            }
            _ => (None, None),
        };
        let mut watcher = socket
            .as_ref()
            .map(|socket| Watcher::start(socket.clone(), watch.clone()));

        let tui = |files: &Files, env: Env, stub: Option<Stub>| -> Result<Live> {
            let args = codex_permission::session(
                self.candidate,
                files,
                &runs,
                self.live,
                socket.as_deref(),
                self.options.model,
                url.as_deref(),
            );
            let name = files.dir.file_name().unwrap_or_default().to_string_lossy();
            let mut live =
                Prepared::new(&name, Agent::Codex, false, files.clone(), args, env, stub)?
                    .start()?;
            live.record_notifications(&files.dir.join(NOTIFIED))?;
            Ok(live)
        };
        let (daemon_env, session_env) = (env()?, env()?);
        let mut daemon = None;
        if self.candidate == Candidate::Daemon {
            // In a new config home, the terminal UI runs its app-server in its own process.
            start_daemon(&daemon_env)?;
            daemon = daemon_socket(&codex).and_then(|socket| fs::canonicalize(socket).ok());
            match &daemon {
                Some(socket) => {
                    // A Unix socket's path has at most 103 bytes on macOS: the watcher connects
                    // through a shorter link.
                    let short = self.work.join(format!("{name}.sock"));
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(socket, &short)?;
                    // Off Unix the session, in a pseudo-terminal, never starts.
                    #[cfg(not(unix))]
                    let _ = socket;
                    watcher = Some(Watcher::start(short, watch.clone()));
                }
                None => eprintln!(
                    "live-check: no background app-server in {}",
                    codex.display()
                ),
            }
        }
        let ready = |live: &mut Live, files: &Files| -> Result<()> {
            let ready = wait_until(live, READY, |live| {
                // The watcher learns of the thread before the prompt starts its turn.
                let watched = !self.candidate.watches()
                    || Observed::read(&runs, &watch, &files.dir)
                        .thread_in(&files.project)
                        .is_some();
                // On the background app-server, the terminal UI asks about new hooks after its
                // composer shows, and starts its thread and a new composer once they are trusted.
                let asked = live.showed(HOOKS) && !live.showed_after(HOOKS, COMPOSER);
                (watched && !asked && live.showed(COMPOSER)).then_some(())
            })?;
            if ready.is_none() {
                eprintln!(
                    "live-check: a session never got ready; see {}",
                    files.screen.display()
                );
            }
            Ok(())
        };
        let mut session = tui(&files, session_env, stub)?;
        ready(&mut session, &files)?;
        // The session without a prompt starts once the first is ready: two terminal UIs that
        // start together in one config home break its database.
        let quiet = match quiet {
            Some(files) => {
                fs::create_dir_all(&files.project)?;
                let stub = self.options.stub(&[])?;
                let env = match &stub {
                    Some(stub) => {
                        Env::Only(queue::stub_environment(&home, &stub.url(), self.path)?)
                    }
                    None => Env::Add(vec![("TERM".to_owned(), TERM.to_owned())]),
                };
                let mut live = tui(&files, env, stub)?;
                ready(&mut live, &files)?;
                Some((live, files))
            }
            None => None,
        };
        let mut observed = Observed::default();
        session.press(codex_permission::prompt().as_bytes())?;
        if !shows(&mut session, TYPED, Duration::from_secs(2))? {
            eprintln!("live-check: session {name}'s composer never showed its prompt");
        }
        // The terminal UI takes text typed at once as a paste, and Enter after it as sending it.
        session.pump_for(Duration::from_millis(500))?;
        session.press(ENTER)?;
        (observed.shown_ms, observed.answered_ms) =
            answer_dialog(&mut session, name, DIALOG, answer.keys(), AFTER)?;
        let ended = session.end()?;
        let quiet = match quiet {
            Some((live, files)) => Some((live.end()?, files)),
            None => None,
        };
        if let Some(watcher) = watcher {
            watcher.stop();
        }
        if self.candidate == Candidate::Daemon {
            stop_daemon(&daemon_env, &codex, daemon.as_deref());
        }
        drop(server);

        let read = Observed::read(&runs, &watch, &files.dir);
        observed = Observed {
            thread: read.thread_in(&files.project),
            shown_ms: observed.shown_ms,
            answered_ms: observed.answered_ms,
            ..read
        };
        let mut lines = verdicts.prompted(name, &observed, answer, &ended.summary());
        if let Some((quiet_session, quiet_files)) = quiet {
            let quiet_observed = Observed {
                thread: observed.thread_in(&quiet_files.project),
                notified: Observed::read(&runs, &watch, &quiet_files.dir).notified,
                ..observed.clone()
            };
            lines.push(
                verdicts.quiet(
                    quiet_files
                        .dir
                        .file_name()
                        .unwrap_or_default()
                        .to_str()
                        .unwrap_or_default(),
                    &observed,
                    &quiet_observed,
                    &quiet_session.summary(),
                ),
            );
        }
        Ok(lines)
    }
}

/// Starts the background app-server of Codex in `env`: Codex installs it from the CLI into the
/// config home and runs it there.
fn start_daemon(env: &Env) -> Result<()> {
    let mut command = Agent::Codex.command();
    env.apply(&mut command);
    command
        .args(["app-server", "daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + READY;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            return Err(format!("the background app-server did not start: {status}").into());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("the background app-server did not start in time".into());
        }
        thread::sleep(TICK);
    }
}

/// Stops the background app-server that runs Codex in `env` on `socket`, and its updater, which
/// outlives it, in the config home `codex`.
fn stop_daemon(env: &Env, codex: &Path, socket: Option<&Path>) {
    let mut command = Agent::Codex.command();
    env.apply(&mut command);
    command
        .args(["app-server", "daemon", "stop"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Err(error) = command.status() {
        eprintln!("live-check: the background app-server did not stop: {error}");
    }
    let updater = fs::read_to_string(codex.join("app-server-daemon").join("daemon-updater.pid"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|pid| pid["pid"].as_u64());
    // Codex keeps its socket in a directory of its own, such as /tmp/codex-daemon-<uid>, and leaves
    // it there.
    if let Some(socket) = socket {
        for path in [socket.to_path_buf(), socket.with_extension("lock")] {
            let _ = fs::remove_file(path);
        }
    }
    if let Some(pid) = updater {
        let killed = std::process::Command::new("kill")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Err(error) = killed {
            eprintln!("live-check: the background app-server's updater did not stop: {error}");
        }
    }
}

/// Runs the session until `found` gives something for it or `limit` passes.
fn wait_until<T>(
    live: &mut Live,
    limit: Duration,
    mut found: impl FnMut(&Live) -> Option<T>,
) -> Result<Option<T>> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(found) = found(live) {
            return Ok(Some(found));
        }
        if !live.pump_for(TICK)? {
            return Ok(found(live));
        }
    }
    Ok(None)
}

/// A client of the app-server that subscribes to every thread it loads and writes each message it
/// gets to a file, with when it came; it answers no request, so every approval stays with the
/// terminal UI. Stopping it ends its connection.
struct Watcher {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<Result<()>>,
}

impl Watcher {
    fn start(socket: PathBuf, log: PathBuf) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::spawn(move || watch(&socket, &log, &stopped));
        Self { stop, thread }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::SeqCst);
        match self.thread.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("live-check: the app-server watcher failed: {error}"),
            Err(_) => eprintln!("live-check: the app-server watcher panicked"),
        }
    }
}

/// How often the watcher lists the app-server's loaded threads.
const LIST_EVERY: Duration = Duration::from_millis(300);

/// The watcher's loop ([`Watcher`]) until `stop` is set or the app-server closes the connection.
fn watch(socket: &Path, log: &Path, stop: &AtomicBool) -> Result<()> {
    let mut transport = Socket::connect(socket)?;
    let mut log = File::create(log)?;
    let mut next_id = 0_u64;
    let mut send = |transport: &mut Socket, method: &str, params: Value| -> Result<u64> {
        let id = next_id;
        next_id += 1;
        transport.send(&json!({ "method": method, "id": id, "params": params }).to_string())?;
        Ok(id)
    };
    let client = json!({ "name": "skald_prototype", "title": "Skald prototype", "version": "0" });
    send(
        &mut transport,
        "initialize",
        json!({ "clientInfo": client, "capabilities": { "experimentalApi": true } }),
    )?;
    let mut initialized = false;
    let mut lists = HashSet::new();
    // The threads it subscribed to, and those it asked to: until a thread's first turn saves it,
    // `thread/resume` fails and the next list asks again.
    let mut subscribed = HashSet::new();
    let mut resumes = HashMap::new();
    let mut listed = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        if initialized && listed.elapsed() >= LIST_EVERY {
            lists.insert(send(&mut transport, "thread/loaded/list", json!({}))?);
            listed = Instant::now();
        }
        let text = match transport.receive(TICK)? {
            Received::Message(text) => text,
            Received::Nothing => continue,
            Received::Closed => break,
        };
        let Ok(message) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        writeln!(log, "{}", watched_line(now_ms(), &message))?;
        if message.get("method").is_some() {
            continue;
        }
        let Some(id) = message["id"].as_u64() else {
            continue;
        };
        if id == 0 && !initialized {
            transport.send(&json!({ "method": "initialized" }).to_string())?;
            initialized = true;
        } else if lists.remove(&id) {
            for thread in message["result"]["data"].as_array().into_iter().flatten() {
                if let Some(thread) = thread.as_str()
                    && !subscribed.contains(thread)
                    && !resumes.values().any(|asked| asked == thread)
                {
                    let params = json!({ "threadId": thread, "excludeTurns": true });
                    let asked = send(&mut transport, "thread/resume", params)?;
                    resumes.insert(asked, thread.to_owned());
                }
            }
        } else if let Some(thread) = resumes.remove(&id)
            && message.get("error").is_none()
        {
            subscribed.insert(thread);
        }
    }
    Ok(())
}
