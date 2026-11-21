//! Codex sessions and the signal through their hooks (`docs/dev/architecture.md`, "Components";
//! `docs/dev/agent-delivery.md`, "Codex hooks"; `docs/protocol/delivery.md`, "Device to agent"):
//!
//! - `skald hook codex` is the handler of Codex's `SessionStart`, `PostToolUse`,
//!   `UserPromptSubmit` and `Stop`, as the agent the session's `skald mcp` registers: the one
//!   `--agent` names, or the default name of [`hook::CODEX`]. Codex runs each hook as its own
//!   child, so the hook's parent is the process that runs the session ([`parent`]).
//! - Each hook registers the session, by its thread, with its folder's space, its agent, its
//!   session file, its config home and its Codex process, as the one whose hook ran last
//!   ([`hook`]); `SessionStart` starts the helper when none runs. In a folder in no space nothing
//!   registers and the hooks print nothing.
//! - A session is busy from a hook of its turn until the `Stop` that ends the turn. It lives while
//!   its Codex process runs ([`runs`]); the helper keeps running meanwhile ([`Served`]).
//! - Each message that waits for the agent goes to one of its live sessions: the one whose hook
//!   ran last before the message was first seen, or, once that one ended, the one whose hook ran
//!   last then. Its signal, [`hook::SIGNAL`], is the only thing of it that goes into a session:
//!   `PostToolUse` and `UserPromptSubmit` add it as context, `Stop` continues the turn with it,
//!   once for the messages whose signal did not go. Otherwise every hook prints nothing.
//! - A hook answers from the local state, through the helper or here, and never waits on the
//!   network: the helper takes in what comes for the sessions' spaces ([`Served`]), and a hook
//!   waits for the helper at most [`PATIENCE`], then prints nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::{Name, SpaceId};

use crate::devices::text;
use crate::mailbox::Came;
use crate::output::{self, Stream};
use crate::state::{CodexSession, State};
use crate::{claude_socket, folders, hook, recv};

/// How long a hook waits for its run, through the helper or here, before it prints nothing.
pub const PATIENCE: Duration = Duration::from_secs(3);

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
    let Ok(space) = folders::working_space(dir) else {
        return Ok(());
    };
    let folder = std::env::current_dir()
        .map_err(|err| format!("cannot read the working directory: {err}"))?;
    let state = State::open(dir).map_err(text)?;
    state.add_agent(space.id, agent.as_str()).map_err(text)?;
    let mut this = CodexSession {
        thread: thread.clone(),
        space: space.id,
        agent: agent.as_str().to_owned(),
        folder,
        file: session.file.clone(),
        home: session.home.clone(),
        process: session.process,
        busy: false,
        hooked: 0,
    };
    if event != hook::Event::SessionStart {
        let signalled = signals(&state, &this)?;
        if !signalled.is_empty() {
            output::write(Stream::Out, output_of(event).as_bytes())
                .map_err(|err| format!("cannot write the signal to the output: {err}"))?;
            (state.signals_given(&signalled, agent.as_str())).map_err(text)?;
            this.busy = true;
        } else {
            // A `Stop` that gives nothing ends the turn.
            this.busy = !matches!(event, hook::Event::Stop { .. });
        }
    }
    state.register_codex(&this).map_err(text)
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

/// The ID of the process that runs this one, as Codex runs a hook; on Windows, the parent of a
/// shell between them.
pub fn parent() -> Option<u32> {
    #[cfg(unix)]
    return Some(std::os::unix::process::parent_id());
    #[cfg(windows)]
    return windows::parent();
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

    /// The parent of this process, or of the shell that is its parent.
    pub fn parent() -> Option<u32> {
        let processes = snapshot()?;
        let parent_of = |pid: u32| {
            (processes.iter())
                .find(|(each, _, _)| *each == pid)
                .map(|(_, parent, name)| (*parent, name.clone()))
        };
        let (parent, _) = parent_of(std::process::id())?;
        match parent_of(parent) {
            Some((grandparent, name))
                if SHELLS.iter().any(|shell| shell.eq_ignore_ascii_case(&name)) =>
            {
                Some(grandparent)
            }
            _ => Some(parent),
        }
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
/// find it in the local state.
pub struct Served {
    settings: claude_socket::Settings,
    /// Whether the registrations may have changed: a command ran.
    reload: bool,
    live: BTreeMap<String, Live>,
    /// When the last intake failed, if it did.
    failed: Option<Instant>,
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
            live: BTreeMap::new(),
            failed: None,
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

    /// Whether the next look has anything to do.
    pub fn looks(&self) -> bool {
        self.reload || !self.live.is_empty()
    }

    /// Serves the sessions registered in the state in `dir`, `came` what came on the connection:
    /// ends those whose Codex process ended; takes in once for each space of a session it has not
    /// taken in for, or that `came` concerns. Gives whether it took in; `None` once a run panicked.
    pub fn look(&mut self, dir: &Path, came: &Came) -> Option<bool> {
        if std::mem::take(&mut self.reload) {
            self.load(dir);
        }
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
        for thread in gone {
            self.live.remove(&thread);
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
    }

    /// Takes in as `agent` in its `folder`, handing nothing over, its output dropped, then goes
    /// back to `dir`: whether the run did not fail; `None` if it panicked.
    fn run(&self, dir: &Path, folder: &Path, agent: &str) -> Option<bool> {
        let Ok(agent) = crate::agents::name(agent) else {
            return Some(false);
        };
        let (storage, speaks, timeout) = self.settings;
        let ran = panic::catch_unwind(AssertUnwindSafe(|| {
            output::into(
                |_, _| Ok(()),
                || {
                    std::env::set_current_dir(folder).map_err(|err| err.to_string())?;
                    recv::hold_run(dir, storage, (speaks, timeout), &agent)
                },
            )
        }));
        let _ = std::env::set_current_dir(dir);
        ran.ok().map(|ran| ran.is_ok())
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
}
