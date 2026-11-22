//! Codex sessions and the signal through their hooks (`docs/dev/architecture.md`, "Components";
//! `docs/dev/agent-delivery.md`, "Codex hooks"; `docs/protocol/delivery.md`, "Device to agent"):
//!
//! - `skald hook codex` is the handler of Codex's `SessionStart`, `PostToolUse`,
//!   `UserPromptSubmit` and `Stop`, as the agent the session's `skald mcp` registers: the one
//!   `--agent` names, or the default name of [`hook::CODEX`]. Codex runs each hook as its own
//!   child, so the hook's parent, past the npm package's launcher and Windows' shells, is the
//!   process that runs the session ([`parent`]).
//! - Each hook registers the session, by its thread, with its folder's space, its agent, its
//!   session file, its config home and its Codex process, as the one whose hook ran last
//!   ([`hook`]); `SessionStart` starts the helper when none runs, without waiting for it. In a
//!   folder in no space nothing registers and the hooks print nothing.
//! - A session is busy from a hook of its turn until the `Stop` that ends the turn. It lives while
//!   its Codex process runs ([`runs`]); the helper keeps running meanwhile ([`Served`]).
//! - Each message that waits for the agent goes to one of its live sessions: the one whose hook
//!   ran last before the first hook of the agent that saw the message, or, once that one ended,
//!   the one whose hook ran last then. Its signal, [`hook::SIGNAL`], is the only thing of it that goes into a session:
//!   `PostToolUse` and `UserPromptSubmit` add it as context, `Stop` continues the turn with it,
//!   once for the messages whose signal did not go. Otherwise every hook prints nothing.
//! - A hook answers from the local state and never waits on the network: it runs without the
//!   lock, which a run on the network may hold; the helper takes in what comes for the sessions'
//!   spaces ([`Served`]). A hook that has not finished within [`PATIENCE`] prints nothing more.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::{Name, SpaceId};

use crate::devices::text;
use crate::mailbox::Came;
use crate::output::{self, Stream};
use crate::state::{CodexSession, State};
#[cfg(unix)]
use crate::codex_app_server;
use crate::{claude_socket, hook, recv};

/// How long a hook waits for its run before it prints nothing.
pub const PATIENCE: Duration = Duration::from_secs(3);

/// The setting that changes [`PATIENCE`], in seconds, for tests.
pub const PATIENCE_SETTING: &str = "SKALD_HOOK_PATIENCE";

/// Codex's hook output limit is about 2,500 tokens; a token takes at least a byte, so an output of
/// at most this many bytes is within it.
pub const LIMIT: usize = 2_500;

/// How often the helper checks that a session's Codex process runs.
const ALIVE_EVERY: Duration = Duration::from_secs(1);

/// How long the helper waits after a failed intake before what comes starts another.
const AFTER_FAILURE: Duration = Duration::from_secs(5);

/// The run of `agent`'s hook on `event` of `session`, its folder the working directory: registers
/// the session; on a hook of a turn, gives the signal if a message waits whose signal goes to this
/// session and did not go yet. In a folder in no space, or without a thread, nothing.
pub fn hook(
    dir: &Path,
    agent: &Name,
    (event, session): (hook::Event, &hook::Session),
) -> Result<(), String> {
    let Some(thread) = &session.id else {
        return Ok(());
    };
    let Some((state, space, folder)) = hook::registering(dir, agent)? else {
        return Ok(());
    };
    let mut this = CodexSession {
        thread: thread.clone(),
        space,
        agent: agent.as_str().to_owned(),
        folder,
        file: session.file.clone(),
        home: session.home.clone(),
        process: session.process,
        busy: false,
        hooked: 0,
    };
    // What waits goes to a session before this one counts as the one whose hook ran last.
    let mut signalled = signals(&state, &this)?;
    if event == hook::Event::SessionStart {
        signalled.clear();
    }
    if !signalled.is_empty() {
        output::write(Stream::Out, output_of(event).as_bytes())
            .map_err(|err| format!("cannot write the signal to the output: {err}"))?;
        (state.signals_given(&signalled, agent.as_str())).map_err(text)?;
    }
    this.busy = busy_after(event, !signalled.is_empty());
    state.register_codex(&this).map_err(text)
}

/// Whether a session is busy after its hook on `event`, which gave the signal if `signalled`: a
/// session starts idle, a hook of its turn makes it busy, and a `Stop` that gives nothing ends the
/// turn.
fn busy_after(event: hook::Event, signalled: bool) -> bool {
    match event {
        hook::Event::SessionStart | hook::Event::SessionEnd => false,
        hook::Event::Stop { .. } => signalled,
        // A prompt comes within a turn; Codex's hooks do not run the handler on it.
        hook::Event::PostToolUse
        | hook::Event::UserPromptSubmit
        | hook::Event::PermissionRequest
        | hook::Event::PermissionPrompt => true,
    }
}

/// The `seq` of each kept message that waits for the agent of `this`, a session whose hook runs,
/// whose signal goes to it and did not go yet. Each waiting message whose signal goes to no live
/// session goes to the live session of the agent whose hook ran last before this run; sessions
/// whose Codex process ended end.
fn signals(state: &State, this: &CodexSession) -> Result<Vec<u64>, String> {
    let agent = this.agent.as_str();
    let signalled: BTreeSet<u64> = state.signalled(agent).map_err(text)?.into_iter().collect();
    let waiting: Vec<u64> = (state.for_agent(this.space, agent, crate::state::unix_now()))
        .map_err(text)?
        .into_iter()
        .map(|(seq, _)| seq)
        .filter(|seq| !signalled.contains(seq))
        .collect();
    if waiting.is_empty() {
        return Ok(Vec::new());
    }
    let mut live = Vec::new();
    for session in state.codex_sessions().map_err(text)? {
        if session.space != this.space || session.agent != agent {
            continue;
        }
        if session.thread == this.thread || runs(session.process) {
            live.push(session.thread);
        } else {
            state.end_codex(&session.thread).map_err(text)?;
        }
    }
    // The sessions come with the one whose hook ran last first.
    let latest = live.first().unwrap_or(&this.thread).clone();
    let going: BTreeMap<u64, String> = state
        .codex_signals(agent)
        .map_err(text)?
        .into_iter()
        .collect();
    let (mut ours, mut moved) = (Vec::new(), Vec::new());
    for seq in waiting {
        let to = match going.get(&seq) {
            Some(to) if live.contains(to) => to,
            _ => {
                moved.push(seq);
                &latest
            }
        };
        if *to == this.thread {
            ours.push(seq);
        }
    }
    state.signal_to(&moved, agent, &latest).map_err(text)?;
    Ok(ours)
}

/// The JSON object a hook on `event` prints to give the signal: on `Stop`, `decision: "block"`
/// with it as `reason`, which continues the turn; otherwise `hookSpecificOutput` with it as
/// `additionalContext`.
fn output_of(event: hook::Event) -> String {
    let printed = match event {
        hook::Event::Stop { .. } => json!({ "decision": "block", "reason": hook::SIGNAL }),
        _ => json!({
            "hookSpecificOutput": {
                "hookEventName": event.name(),
                "additionalContext": hook::SIGNAL,
            }
        }),
    }
    .to_string();
    debug_assert!(printed.len() <= LIMIT, "{printed}");
    printed
}

/// The setting by which the launcher of the npm package (`npm/skald.js`) tells the `skald` it runs
/// its own process ID and its parent's, space-separated.
pub const LAUNCHER: &str = "SKALD_LAUNCHER";

/// The ID of the process that runs this one, as Codex runs a hook: past the npm package's launcher
/// ([`LAUNCHER`]) and, on Windows, the shells between them.
pub fn parent() -> Option<u32> {
    let launcher = std::env::var(LAUNCHER)
        .ok()
        .and_then(|given| launcher_of(&given));
    #[cfg(unix)]
    {
        let parent = std::os::unix::process::parent_id();
        Some(match launcher {
            Some((launcher, above)) if launcher == parent => above,
            _ => parent,
        })
    }
    #[cfg(windows)]
    return windows::parent(launcher.map(|(launcher, _)| launcher));
}

/// The launcher's process ID and its parent's, as [`LAUNCHER`] gives them.
fn launcher_of(given: &str) -> Option<(u32, u32)> {
    let (launcher, above) = given.trim().split_once(' ')?;
    Some((launcher.parse().ok()?, above.trim().parse().ok()?))
}

/// Whether the process `process` runs; one not known counts as running.
pub fn runs(process: Option<u32>) -> bool {
    let Some(process) = process else {
        return true;
    };
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(process) else {
            return false;
        };
        // SAFETY: signal 0 only checks that the process exists.
        let sent = unsafe { libc::kill(pid, 0) };
        sent == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(windows)]
    windows::runs(process)
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, GetLastError, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };

    /// The shells a hook's command may run in, whose parent then runs the session.
    const SHELLS: [&str; 3] = ["cmd.exe", "powershell.exe", "pwsh.exe"];

    /// The nearest process above this one that is neither a shell nor `launcher`.
    pub fn parent(launcher: Option<u32>) -> Option<u32> {
        let processes = snapshot()?;
        let parent_of = |pid: u32| {
            (processes.iter())
                .find(|(each, _, _)| *each == pid)
                .map(|(_, parent, _)| *parent)
        };
        let skipped = |pid: u32| {
            Some(pid) == launcher
                || (processes.iter()).any(|(each, _, name)| {
                    *each == pid && SHELLS.iter().any(|shell| shell.eq_ignore_ascii_case(name))
                })
        };
        let mut process = parent_of(std::process::id())?;
        // Within as many steps as there are processes, even if reused IDs make a loop.
        for _ in 0..processes.len() {
            if !skipped(process) {
                break;
            }
            match parent_of(process) {
                Some(above) if above != process => process = above,
                _ => break,
            }
        }
        Some(process)
    }

    /// Each process's ID, its parent's and its executable's name.
    fn snapshot() -> Option<Vec<(u32, u32, String)>> {
        // SAFETY: it takes no pointer; the snapshot is closed below.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        // SAFETY: an entry of plain integers and characters, all zeros a valid one.
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).ok()?;
        let mut processes = Vec::new();
        // SAFETY: `entry` is an entry with its size set, for the snapshot taken.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            let length = (entry.szExeFile.iter())
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            processes.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
            // SAFETY: as for the first entry.
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        // SAFETY: the snapshot taken above, closed once.
        unsafe { CloseHandle(snapshot) };
        Some(processes)
    }

    /// Whether the process `pid` runs.
    pub fn runs(pid: u32) -> bool {
        let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        // SAFETY: it takes no pointer; the handle is closed below.
        let process = unsafe { OpenProcess(access, 0, pid) };
        if process.is_null() {
            // SAFETY: it reads this thread's last error.
            return unsafe { GetLastError() } == ERROR_ACCESS_DENIED;
        }
        // SAFETY: the handle opened above, with the right to wait on it.
        let running = unsafe { WaitForSingleObject(process, 0) } == WAIT_TIMEOUT;
        // SAFETY: the handle opened above, closed once.
        unsafe { CloseHandle(process) };
        running
    }
}

/// The Codex sessions the helper serves: it ends those whose Codex process ended, keeps running
/// while one with a known process lives, and takes in what comes for their spaces, so their hooks
/// find it in the local state. Hooks register without it, so it reads the registrations again at
/// least every [`ALIVE_EVERY`].
pub struct Served {
    settings: claude_socket::Settings,
    /// Whether the registrations may have changed: a command ran.
    reload: bool,
    /// When it last read the registrations.
    loaded: Option<Instant>,
    live: BTreeMap<String, Live>,
    /// When the last intake failed, if it did.
    failed: Option<Instant>,
    /// The clients on the sessions' app-servers.
    #[cfg(unix)]
    watchers: codex_app_server::Watchers,
}

/// A registered session the helper serves: when its process was last seen running, and whether
/// the helper took in for it yet.
struct Live {
    session: CodexSession,
    seen: Option<Instant>,
    taken: bool,
}

impl Live {
    /// Whether its Codex process runs, as of at most [`ALIVE_EVERY`] ago.
    fn runs(&mut self) -> bool {
        if self.seen.is_some_and(|seen| seen.elapsed() < ALIVE_EVERY) {
            return true;
        }
        let runs = runs(self.session.process);
        self.seen = runs.then(Instant::now);
        runs
    }
}

impl Served {
    pub fn new(settings: claude_socket::Settings) -> Self {
        Served {
            settings,
            reload: true,
            loaded: None,
            live: BTreeMap::new(),
            failed: None,
            #[cfg(unix)]
            watchers: codex_app_server::Watchers::default(),
        }
    }

    /// Notes that a command ran.
    pub fn ran(&mut self) {
        self.reload = true;
    }

    /// Whether a session with a known Codex process lives, as of the last look.
    pub fn serving(&self) -> bool {
        (self.live.values()).any(|live| live.session.process.is_some())
    }

    /// Whether a session is registered in the state in `dir`, read again if due.
    pub fn looks(&mut self, dir: &Path) -> bool {
        let due = (self.loaded).is_none_or(|loaded| loaded.elapsed() >= ALIVE_EVERY);
        if std::mem::take(&mut self.reload) || due {
            self.load(dir);
        }
        !self.live.is_empty()
    }

    /// Serves the sessions registered in the state in `dir` ([`Served::looks`]), `came` what came
    /// on the connection: ends those whose Codex process ended; takes in once for each space of a session it has not
    /// taken in for, or that `came` concerns. Gives whether it took in; `None` once a run panicked.
    pub fn look(&mut self, dir: &Path, came: &Came) -> Option<bool> {
        self.looks(dir);
        let gone: Vec<String> = (self.live.iter_mut())
            .filter_map(|(thread, live)| (!live.runs()).then(|| thread.clone()))
            .collect();
        if !gone.is_empty()
            && let Ok(state) = State::open(dir)
        {
            for thread in &gone {
                let _ = state.end_codex(thread);
            }
        }
        if !gone.is_empty() {
            for thread in gone {
                self.live.remove(&thread);
            }
            self.watch(dir);
        }
        let retry = (self.failed).is_none_or(|failed| failed.elapsed() >= AFTER_FAILURE);
        let mut runs: BTreeMap<SpaceId, (String, PathBuf)> = BTreeMap::new();
        for live in self.live.values_mut() {
            let session = &live.session;
            if !std::mem::replace(&mut live.taken, true)
                || (retry && came.concerns(Some(session.space)))
            {
                (runs.entry(session.space))
                    .or_insert_with(|| (session.agent.clone(), session.folder.clone()));
            }
        }
        for (agent, folder) in runs.values() {
            match self.run(dir, folder, agent)? {
                true => self.failed = None,
                false => self.failed = Some(Instant::now()),
            }
        }
        Some(!runs.is_empty())
    }

    /// Takes the registrations from the state in `dir`, keeping what it has of each it served.
    fn load(&mut self, dir: &Path) {
        self.loaded = Some(Instant::now());
        let Ok(registered) = State::open(dir).and_then(|state| state.codex_sessions()) else {
            return;
        };
        let mut live = std::mem::take(&mut self.live);
        for session in registered {
            let thread = session.thread.clone();
            let served = match live.remove(&thread) {
                Some(kept) => Live { session, ..kept },
                None => Live {
                    session,
                    seen: None,
                    taken: false,
                },
            };
            self.live.insert(thread, served);
        }
        self.watch(dir);
    }

    /// Keeps a client on the app-server of each live session's config home, by the state in
    /// `dir` ([`codex_app_server`]).
    fn watch(&mut self, dir: &Path) {
        #[cfg(unix)]
        (self.watchers).keep(dir, self.live.values().map(|live| &live.session));
        #[cfg(windows)]
        let _ = dir;
    }

    /// Takes in as `agent` in its `folder`, handing nothing over ([`claude_socket::run_as`]).
    fn run(&self, dir: &Path, folder: &Path, agent: &str) -> Option<bool> {
        claude_socket::run_as(
            self.settings,
            dir,
            (folder, agent),
            |storage, settings, agent| recv::hold_run(dir, storage, settings, agent),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The signal's output on each event is one JSON object within the limit, holding the signal
    /// where Codex takes it: `Stop`'s `reason`, which continues the turn; another event's
    /// `additionalContext`.
    #[test]
    fn signal_goes_where_codex_takes_it_within_the_limit() {
        for event in [
            hook::Event::PostToolUse,
            hook::Event::UserPromptSubmit,
            hook::Event::Stop { continued: false },
        ] {
            let printed = output_of(event);
            assert!(printed.len() <= LIMIT, "{printed}");
            let output: serde_json::Value = serde_json::from_str(&printed).unwrap();
            let signal = match event {
                hook::Event::Stop { .. } => {
                    assert_eq!(output["decision"], "block");
                    &output["reason"]
                }
                _ => {
                    assert_eq!(output["hookSpecificOutput"]["hookEventName"], event.name());
                    &output["hookSpecificOutput"]["additionalContext"]
                }
            };
            assert_eq!(signal.as_str(), Some(hook::SIGNAL));
        }
    }

    /// This process runs, and its parent is the one that started it.
    #[test]
    fn a_running_process_runs() {
        assert!(runs(Some(std::process::id())));
        assert!(runs(parent()));
        assert!(runs(None));
    }

    /// A session is idle after `SessionStart` and after a `Stop` that gives nothing, and busy after
    /// any other hook of its turn.
    #[test]
    fn busy_from_a_hook_of_its_turn_until_a_stop_that_gives_nothing() {
        for signalled in [false, true] {
            assert!(!busy_after(hook::Event::SessionStart, signalled));
            assert!(busy_after(hook::Event::PostToolUse, signalled));
            assert!(busy_after(hook::Event::UserPromptSubmit, signalled));
            for continued in [false, true] {
                let stop = hook::Event::Stop { continued };
                assert_eq!(busy_after(stop, signalled), signalled, "{stop:?}");
            }
        }
    }

    /// The launcher's setting gives its process ID and its parent's; anything else gives none.
    #[test]
    fn launcher_setting_gives_both_processes() {
        assert_eq!(launcher_of("41 7"), Some((41, 7)));
        assert_eq!(launcher_of(" 41 7\n"), Some((41, 7)));
        for given in ["", "41", "41 x", "-1 7"] {
            assert_eq!(launcher_of(given), None, "{given:?}");
        }
    }

    /// The setting with which a run of this test acts in a run of it: prints its [`parent`], as
    /// `parent`; runs this test as `parent` through the launcher's setting, as `launcher`.
    #[cfg(windows)]
    const PROBE: &str = "SKALD_TEST_PARENT_PROBE";

    /// Run through each shell a hook's command may run in, as Codex runs it, nested too, and
    /// through a launcher as the npm package's, a run's parent is this process. A shell not
    /// installed is skipped.
    #[cfg(windows)]
    #[test]
    fn parent_is_past_each_shell_and_the_launcher() {
        let exe = std::env::current_exe().unwrap();
        let test = [
            "codex::tests::parent_is_past_each_shell_and_the_launcher",
            "--exact",
            "--nocapture",
        ];
        match std::env::var(PROBE).as_deref() {
            Ok("parent") => {
                println!("parent: {}", parent().unwrap_or_default());
                return;
            }
            Ok(_) => {
                // Windows walks past the launcher by its own process ID alone.
                let given = format!("{} 0", std::process::id());
                let status = (std::process::Command::new(&exe).args(test))
                    .env(PROBE, "parent")
                    .env(LAUNCHER, given)
                    .status()
                    .unwrap();
                assert!(status.success());
                return;
            }
            Err(_) => {}
        }
        let quoted = format!("& '{}' {}", exe.display(), test.join(" "));
        let runs: [(&str, &[&str], &str); 5] = [
            ("cmd", &["/c"], "parent"),
            ("powershell", &["-NoProfile", "-Command"], "parent"),
            ("pwsh", &["-NoProfile", "-Command"], "parent"),
            (
                "cmd",
                &["/c", "powershell", "-NoProfile", "-Command"],
                "parent",
            ),
            ("cmd", &["/c"], "launcher"),
        ];
        for (shell, args, mode) in runs {
            let mut command = std::process::Command::new(shell);
            command.args(args).env(PROBE, mode);
            match args.last() {
                Some(&"/c") => command.arg(&exe).args(test),
                _ => command.arg(&quoted),
            };
            let output = match command.output() {
                Ok(output) => output,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => panic!("{shell}: {err}"),
            };
            let printed = String::from_utf8_lossy(&output.stdout);
            let parent = (printed.lines())
                .find_map(|line| line.strip_prefix("parent: "))
                .unwrap_or_else(|| panic!("{shell} {args:?} {mode}: {printed}"));
            let expected = std::process::id().to_string();
            assert_eq!(parent.trim(), expected, "{shell} {args:?} {mode}");
        }
    }
}
