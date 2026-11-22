//! What the tests of the helper share: runs of a copy of `skald` of Ada's machine's own, which may
//! go through a helper, and the processes of that copy, found by their executable. The copy is
//! `bin/skald` under the machine's directory, whose name no other machine has, so a process whose
//! executable's path ends so is one these tests started, directly or through a command. Each one
//! left is killed on drop, before the machine's directory is removed.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{ChannelId, MessageId, Role};

use crate::cloud::WAIT;
use crate::common::CLIENT;
use crate::fixtures::{SEND, TIMEOUT_SETTING, msg_id};
use crate::kit::{RECV, Space, create_space, received, with_channel};
use crate::machine::{Finished, Machine, Run};
use crate::talk::{Talk, messages_sent};
use crate::world::CLIENT_B;

/// The command that runs the helper.
pub const HELPER: &str = "helper";
/// The flag of one-shot mode.
pub const ONE_SHOT: &str = "--one-shot";
/// The setting of one-shot mode.
pub use crate::machine::ONE_SHOT as ONE_SHOT_SETTING;
/// The setting of the helper's idle time, in seconds.
pub const IDLE_SETTING: &str = "SKALD_HELPER_IDLE";
/// The setting that names the file of the client's timing marks (`src/timing.rs`).
pub const TIMING_LOG: &str = "SKALD_TIMING_LOG";
/// The setting of the version a run acts as, for tests.
pub const VERSION_SETTING: &str = "SKALD_VERSION";
/// A request timeout, in seconds, long enough for the helper a command starts on a loaded machine
/// to listen before the command runs without it.
pub const START_TIMEOUT: &str = "30";
/// The flag of a run's request timeout, in seconds.
pub const TIMEOUT_FLAG: &str = "--timeout";
/// The version of the `skald` these tests build, and of its copy on disk.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The helper's idle time in every run of these tests unless a test sets its own: longer than any
/// test waits between two commands, and short enough that a helper a test leaves exits soon.
pub const IDLE: Duration = Duration::from_secs(30);
/// The request that opens a WebSocket, as its line starts.
pub const WEBSOCKET: &str = "GET /ws ";

/// How long a test waits for a helper to exit.
pub const EXIT_LIMIT: Duration = Duration::from_secs(30);

/// The idle time, in seconds, of a helper that must not exit idle while a test waits for it to
/// exit otherwise: well above [`EXIT_LIMIT`].
pub const LONG_IDLE: &str = "600";

/// The runs of Ada's machine's copy of `skald`, and its processes.
pub struct Helped {
    /// The copy.
    exe: PathBuf,
    /// The end of the copy's path that no other machine's executable has: the machine's directory,
    /// `bin` and the file.
    tail: PathBuf,
    /// The machine's home: the working directory of a run that names none.
    home: PathBuf,
    /// The settings of the machine's runs.
    env: Vec<(OsString, OsString)>,
}

impl Helped {
    /// Copies `skald` to `machine`'s `bin/`; its runs from then on have `machine`'s settings, but
    /// not the one-shot setting.
    pub fn new(machine: &Machine) -> Self {
        let root = machine.root();
        let file = format!("skald{}", std::env::consts::EXE_SUFFIX);
        let exe = root.join("bin").join(&file);
        fs::copy(CLIENT, &exe).expect("copy skald to the machine");
        let tail: PathBuf = [
            root.file_name()
                .expect("the machine's directory has a name"),
            "bin".as_ref(),
            file.as_ref(),
        ]
        .iter()
        .collect();
        let home = root.join("home");
        let env = machine
            .command_in(&home, &[])
            .get_envs()
            .filter_map(|(name, value)| Some((name.to_owned(), value?.to_owned())))
            .collect();
        Helped {
            exe,
            tail,
            home,
            env,
        }
    }

    /// Starts the copy with `args` in `dir`, with `settings` beside the machine's.
    pub fn start_in(&self, dir: &Path, settings: &[(&str, &str)], args: &[&str]) -> Run {
        Run::spawn(&mut self.command_in(dir, settings, args))
    }

    /// The copy with `args` in `dir`, with `settings` beside the machine's.
    pub fn command_in(&self, dir: &Path, settings: &[(&str, &str)], args: &[&str]) -> Command {
        let mut command = Command::new(&self.exe);
        command
            .env_clear()
            .current_dir(dir)
            .args(args)
            .envs(self.env.iter().map(|(name, value)| (name, value)))
            .env_remove(ONE_SHOT_SETTING)
            .env(IDLE_SETTING, IDLE.as_secs().to_string())
            .envs(settings.iter().copied());
        // The client's opt-in timing marks (`src/timing.rs`), if the tests' run has them.
        if let Some(log) = std::env::var_os(TIMING_LOG) {
            command.env(TIMING_LOG, log);
        }
        command
    }

    /// Runs the copy with `args` in the machine's home, its standard output a pipe whose reading
    /// end is closed, so each write to it fails; gives how it finished, its standard output empty.
    pub fn run_with_closed_output(&self, args: &[&str]) -> Finished {
        let (reader, writer) = std::io::pipe().expect("a pipe");
        drop(reader);
        let output = self
            .command_in(&self.home, &[], args)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(Stdio::piped())
            .output()
            .expect("run skald");
        Finished {
            success: output.status.success(),
            stdout: String::new(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// As [`Helped::start_in`], in the machine's home.
    pub fn start(&self, settings: &[(&str, &str)], args: &[&str]) -> Run {
        self.start_in(&self.home, settings, args)
    }

    /// Runs the copy with `args` in the machine's home; gives how it finished.
    pub fn run(&self, args: &[&str]) -> Finished {
        self.start(&[], args).finish()
    }

    /// As [`Helped::run`], with `settings`.
    pub fn run_with(&self, settings: &[(&str, &str)], args: &[&str]) -> Finished {
        self.start(settings, args).finish()
    }

    /// As [`Helped::run`], in `dir`.
    pub fn run_in(&self, dir: &Path, args: &[&str]) -> Finished {
        self.start_in(dir, &[], args).finish()
    }

    /// Runs `skald send` of `text` to the talk's channel, with `settings`.
    pub fn send_with(&self, talk: &Talk, settings: &[(&str, &str)], text: &str) -> Finished {
        let channel = talk.channel.to_string();
        self.run_with(settings, &[SEND, "--all", "--channel", &channel, text])
    }

    /// As [`Helped::send_with`], without settings; fails unless it succeeds.
    #[track_caller]
    pub fn sent(&self, talk: &Talk, text: &str) -> Finished {
        let finished = self.send_with(talk, &[], text);
        assert!(finished.success, "{text:?} not sent: {finished:?}");
        finished
    }

    /// As [`Helped::sent`], with `settings`; gives the one helper running then.
    #[track_caller]
    pub fn helper_after_sending(&self, talk: &Talk, settings: &[(&str, &str)], text: &str) -> u32 {
        let finished = self.send_with(talk, settings, text);
        assert!(finished.success, "{text:?} not sent: {finished:?}");
        self.one_helper()
    }

    /// Runs `skald recv`; fails unless it succeeds.
    #[track_caller]
    pub fn received(&self) -> Finished {
        let finished = self.run(&[RECV]);
        assert!(finished.success, "{finished:?}");
        finished
    }

    /// The processes of the copy running now: with no run of a test going, its helpers.
    pub fn helpers(&self) -> Vec<u32> {
        let mut found = running(&self.tail);
        found.sort_unstable();
        found
    }

    /// The one helper running now; fails unless there is one.
    #[track_caller]
    pub fn one_helper(&self) -> u32 {
        match self.helpers().as_slice() {
            [one] => *one,
            other => panic!("not one helper running: {other:?}"),
        }
    }

    /// Fails unless no helper runs now.
    #[track_caller]
    pub fn assert_none(&self) {
        let running = self.helpers();
        assert!(running.is_empty(), "a helper runs: {running:?}");
    }

    /// Waits for up to `limit` until `pid` no longer runs; fails if it still does.
    #[track_caller]
    pub fn await_exit(&self, pid: u32, limit: Duration) {
        let deadline = Instant::now() + limit;
        while self.helpers().contains(&pid) {
            assert!(
                Instant::now() < deadline,
                "the helper {pid} still runs after {limit:?}"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// Waits for up to [`WAIT`] until exactly one helper has run for `steady` on end; gives it.
    #[track_caller]
    pub fn await_one_steady(&self, steady: Duration) -> u32 {
        let deadline = Instant::now() + WAIT;
        let mut since: Option<(u32, Instant)> = None;
        loop {
            let running = self.helpers();
            since = match (running.as_slice(), since) {
                ([one], Some((pid, from))) if *one == pid => {
                    if from.elapsed() >= steady {
                        return pid;
                    }
                    Some((pid, from))
                }
                ([one], _) => Some((*one, Instant::now())),
                _ => None,
            };
            assert!(
                Instant::now() < deadline,
                "not one helper for {steady:?} on end: {running:?}"
            );
            thread::sleep(Duration::from_millis(100));
        }
    }

    /// The copy.
    pub fn exe(&self) -> &Path {
        &self.exe
    }

    /// Replaces the copy with `with`, as `skald update` replaces its binary while runs of it go on
    /// (`update.rs`): a staged copy is renamed over it, once the old file is set aside on
    /// Windows, which does not replace a running program's file.
    pub fn replace(&self, with: &Path) {
        let suffix = std::env::consts::EXE_SUFFIX;
        let staged = self.exe.with_file_name(format!("staged-skald{suffix}"));
        fs::copy(with, &staged).expect("stage the new copy");
        if cfg!(windows) {
            let aside = self.exe.with_file_name(format!("old-skald{suffix}"));
            fs::rename(&self.exe, aside).expect("set the old copy aside");
        }
        fs::rename(&staged, &self.exe).expect("replace the copy");
    }

    /// Kills the process `pid` of the copy and waits until it no longer runs.
    pub fn kill(&self, pid: u32) {
        kill(pid);
        self.await_exit(pid, EXIT_LIMIT);
    }
}

/// Kills every process of the copy still running.
impl Drop for Helped {
    fn drop(&mut self) {
        let deadline = Instant::now() + EXIT_LIMIT;
        loop {
            let left = running(&self.tail);
            if left.is_empty() {
                return;
            }
            if Instant::now() > deadline {
                let failed = format!("the processes {left:?} of {:?} still run", self.exe);
                if thread::panicking() {
                    eprintln!("{failed}");
                    return;
                }
                panic!("{failed}");
            }
            for pid in left {
                kill(pid);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// The settings of a run with a request timeout of `seconds`.
pub fn timeout_of(seconds: &str) -> [(&'static str, &str); 1] {
    [(TIMEOUT_SETTING, seconds)]
}

/// Fails unless Bob's device reads the messages Ada's machine sent to the talk's channel, each
/// once: `texts`, in order, with the counters from 1.
#[track_caller]
pub fn bob_reads(talk: &mut Talk, texts: &[&str]) {
    let sent: Vec<MessageId> = messages_sent(&talk.front, talk.channel, 0)
        .iter()
        .map(msg_id)
        .collect();
    assert_eq!(sent.len(), texts.len(), "message envelopes sent: {sent:?}");
    let read: Vec<(MessageId, u64, &str)> = (sent.into_iter().zip(1..).zip(texts))
        .map(|((id, counter), text)| (id, counter, *text))
        .collect();
    talk.bob_reads_from_ada(&read);
}

/// Ada creates `clientb`, with a channel that holds Bob's device; Ada's machine follows it, and
/// Bob's device takes in its mailbox. Gives the space and the channel.
pub fn with_client_b(talk: &mut Talk) -> (Space, ChannelId) {
    let cloud = &talk.world.cloud;
    let bob = &talk.played[0].person;
    let space = create_space(cloud, &talk.ada, CLIENT_B);
    let channel = with_channel(cloud, space.id, &talk.ada, &[(bob, Role::Member)]);
    received(&talk.ada.machine);
    talk.played[0].take_in(cloud);
    (space, channel)
}

/// How many times `shown` holds `text`.
pub fn times(shown: &str, text: &str) -> usize {
    shown.matches(text).count()
}

/// A version older than [`VERSION`].
pub fn older() -> String {
    let mut version = semver::Version::parse(VERSION).expect("a semantic version");
    version.pre = semver::Prerelease::EMPTY;
    if version.patch > 0 {
        version.patch -= 1;
    } else if version.minor > 0 {
        version.minor -= 1;
    } else {
        version.major = version.major.checked_sub(1).expect("an older version");
    }
    version.to_string()
}

/// A version newer than [`VERSION`]: its next minor version.
pub fn newer() -> String {
    let version = semver::Version::parse(VERSION).expect("a semantic version");
    semver::Version::new(version.major, version.minor + 1, 0).to_string()
}

/// The processes whose executable's path ends with `tail`.
#[cfg(target_os = "linux")]
fn running(tail: &Path) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            // A process that has exited has no executable, a zombie included.
            let exe = fs::read_link(entry.path().join("exe")).ok()?;
            exe.ends_with(tail).then_some(pid)
        })
        .collect()
}

/// The processes whose executable's path ends with `tail`.
#[cfg(target_os = "macos")]
fn running(tail: &Path) -> Vec<u32> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: with no buffer, it only counts the processes.
    let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
    let Ok(count) = usize::try_from(count) else {
        return Vec::new();
    };
    // Room for processes started meanwhile.
    let mut pids: Vec<libc::c_int> = vec![0; count + 256];
    let size = libc::c_int::try_from(std::mem::size_of_val(pids.as_slice()))
        .expect("the size of the process list");
    // SAFETY: the buffer holds `size` bytes.
    let listed = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
    let listed = usize::try_from(listed).unwrap_or(0).min(pids.len());
    pids[..listed]
        .iter()
        .filter_map(|&pid| {
            let mut path = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
            let capacity = u32::try_from(path.len()).expect("the size of a path");
            // SAFETY: the buffer holds `capacity` bytes.
            let length = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), capacity) };
            let length = usize::try_from(length).ok().filter(|length| *length > 0)?;
            let exe = Path::new(OsStr::from_bytes(&path[..length]));
            if exe.ends_with(tail) {
                u32::try_from(pid).ok()
            } else {
                None
            }
        })
        .collect()
}

/// The processes whose executable's path ends with `tail`, from a snapshot of the system's: a
/// helper with a short idle time is listed before it exits, however loaded the machine.
#[cfg(windows)]
fn running(tail: &Path) -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: it takes no pointer; the snapshot is closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    assert!(
        snapshot != INVALID_HANDLE_VALUE,
        "listing the processes failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: an entry of plain integers and characters, all zeros a valid one.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).expect("an entry's size");
    let mut pids = Vec::new();
    // SAFETY: `entry` is an entry with its size set, for the snapshot taken.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        pids.push(entry.th32ProcessID);
        // SAFETY: as for the first entry.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot taken above, closed once.
    unsafe { CloseHandle(snapshot) };
    pids.into_iter()
        .filter(|&pid| image_of(pid).is_some_and(|exe| exe.ends_with(tail)))
        .collect()
}

/// The path of the executable of the process `pid`, if it runs and this one may ask.
#[cfg(windows)]
fn image_of(pid: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    // SAFETY: it takes no pointer; the handle is closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut path = vec![0_u16; 32 << 10];
    let mut length = u32::try_from(path.len()).expect("a path's room");
    // SAFETY: `path` holds `length` characters, and `length` is the room it gives.
    let named = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
    } != 0;
    // SAFETY: the handle opened above, closed once.
    unsafe { CloseHandle(process) };
    if !named {
        return None;
    }
    let wide = path.get(..usize::try_from(length).ok()?)?;
    Some(PathBuf::from(OsString::from_wide(wide)))
}

#[cfg(unix)]
fn kill(pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    // SAFETY: a signal to a process of the copy, which these tests started.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn kill(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .output();
}
