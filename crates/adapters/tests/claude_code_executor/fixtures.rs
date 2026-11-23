//! What the tests share: scratch directories, the device's environment, a stand-in `claude`, a
//! local witness server, finding the real Claude Code, and running an executor within a limit.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use skald_adapters::claude_code::Executor;
use skald_adapters::executor::{Mode, Outcome};

use crate::stub::PLACEHOLDER;

/// The Claude Code version the live checks of `read-only` ran on (`docs/dev/agent-delivery.md`,
/// "Checks before implementation").
pub const CHECKED: &str = "2.1.328";

/// Variables of the device's environment that Claude Code needs neither to run nor to sign in: no
/// executor passes them on.
pub const MARKERS: [&str; 2] = ["SKALD_E2E_MARKER", "GITHUB_TOKEN"];

/// The setting that turns a skipped e2e test into a failure.
const REQUIRED: &str = "SKALD_E2E_REQUIRED";

/// How long an executor may run: no person answers it.
const RUN_LIMIT: Duration = Duration::from_secs(180);

/// Sets each variable of `names` in this process's environment too, so a run that inherits this
/// process's environment instead of getting only the device's gets them.
pub fn set_in_this_process(names: &[&str]) {
    for name in names {
        // SAFETY: these tests' processes read and write their environment only through `std`,
        // which serializes it.
        unsafe { env::set_var(name, token(name)) };
    }
}

/// A unique piece of text, which shows where it went.
pub fn token(of: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let nanos = (SystemTime::now().duration_since(UNIX_EPOCH))
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{of}-{:x}{nanos:x}{next}", std::process::id())
}

/// `path` as Claude Code sees its directory: symlinks resolved and, on Windows, without the `\\?\`
/// prefix.
pub fn plain(path: &Path) -> PathBuf {
    let resolved = fs::canonicalize(path).expect("resolve a path");
    let text = resolved.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if cfg!(windows) => PathBuf::from(rest),
        _ => resolved,
    }
}

/// A directory of the test's own in the temporary directory; removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new() -> Self {
        let made = env::temp_dir().join(token("skx"));
        fs::create_dir_all(&made).expect("make a scratch directory");
        Scratch(plain(&made))
    }

    /// The directory `name` in it, made.
    pub fn dir(&self, name: &str) -> PathBuf {
        let dir = self.0.join(name);
        fs::create_dir_all(&dir).expect("make a directory");
        dir
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The device's environment an executor starts from: what this process has to run programs, the
/// `home` and the config home `config`, a sign-in by API key on the model stub at `stub`, and the
/// [`MARKERS`], each set to `marker(name)`.
pub fn device_environment(
    home: &Path,
    config: &Path,
    stub: &str,
    marker: impl Fn(&str) -> String,
) -> Vec<(OsString, OsString)> {
    let running = [
        "PATH",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
    ];
    let mut vars: Vec<(OsString, OsString)> = (running.iter())
        .filter_map(|name| Some(((*name).into(), env::var_os(name)?)))
        .collect();
    let homes: &[&str] = if cfg!(windows) {
        &["HOME", "USERPROFILE"]
    } else {
        &["HOME"]
    };
    vars.extend(homes.iter().map(|name| ((*name).into(), home.into())));
    vars.push(("CLAUDE_CONFIG_DIR".into(), config.into()));
    vars.push(("ANTHROPIC_BASE_URL".into(), stub.into()));
    vars.push(("ANTHROPIC_API_KEY".into(), PLACEHOLDER.into()));
    vars.extend(
        MARKERS
            .iter()
            .map(|name| ((*name).into(), marker(name).into())),
    );
    vars
}

/// Runs `executor` for `folder`, `mode` and `prompt` on a thread of its own; its outcome, or why it
/// refused, as it says. Fails unless it ends within [`RUN_LIMIT`].
#[track_caller]
pub fn run(executor: Executor, folder: &Path, mode: Mode, prompt: &str) -> Result<Outcome, String> {
    let (done, ended) = mpsc::channel();
    let (folder, prompt) = (folder.to_owned(), prompt.to_owned());
    thread::spawn(move || {
        let outcome = executor.run(&folder, mode, &prompt);
        let _ = done.send(outcome.map_err(|refused| refused.to_string()));
    });
    (ended.recv_timeout(RUN_LIMIT))
        .unwrap_or_else(|_| panic!("the executor did not end within {RUN_LIMIT:?}"))
}

/// Claude Code's binary on `PATH`, if any. Without it, says that the test is skipped, or fails if
/// [`REQUIRED`] is set.
pub fn claude_code() -> Option<PathBuf> {
    let names = if cfg!(windows) {
        vec!["claude.exe", "claude.cmd"]
    } else {
        vec!["claude"]
    };
    let path = env::var_os("PATH").unwrap_or_default();
    for dir in env::split_paths(&path) {
        if let Some(found) = names
            .iter()
            .map(|name| dir.join(name))
            .find(|at| at.is_file())
        {
            return Some(found);
        }
    }
    let required = env::var(REQUIRED).unwrap_or_default();
    if !matches!(required.as_str(), "" | "0") {
        panic!("{REQUIRED} is set, and Claude Code is not on PATH");
    }
    eprintln!("skipped: no Claude Code on PATH");
    None
}

/// A stand-in `claude`: `-v` or `--version` prints its version line and exits with its code; any
/// other run is a session, which records its directory and which [`MARKERS`] it got, then prints a
/// result.
pub struct StandIn {
    pub path: PathBuf,
    records: PathBuf,
}

impl StandIn {
    /// Writes a stand-in into `scratch` whose version run prints `version`, if any, and exits with
    /// `code`.
    pub fn new(scratch: &Scratch, version: Option<&str>, code: u8) -> Self {
        let records = scratch.dir("stand-in-records");
        let dir = scratch.dir("stand-in");
        let rec = records.to_string_lossy().into_owned();
        let path = if cfg!(windows) {
            let mut script = String::from("@echo off\r\n");
            script +=
                "if \"%~1\"==\"--version\" goto version\r\nif \"%~1\"==\"-v\" goto version\r\n";
            script += &format!("cd > \"{rec}\\cwd\"\r\n");
            for name in MARKERS {
                script += &format!("if defined {name} type nul > \"{rec}\\{name}\"\r\n");
            }
            script += &format!("type nul > \"{rec}\\session\"\r\necho {RESULT}\r\nexit /b 0\r\n");
            script += ":version\r\n";
            if let Some(version) = version {
                script += &format!("echo {version}\r\n");
            }
            script += &format!("exit /b {code}\r\n");
            let path = dir.join("claude.cmd");
            fs::write(&path, script).expect("write the stand-in");
            path
        } else {
            let mut script = String::from("#!/bin/sh\nfor arg in \"$@\"; do\n");
            script += "  case \"$arg\" in -v|--version)\n";
            if let Some(version) = version {
                script += &format!("    printf '%s\\n' '{version}'\n");
            }
            script += &format!("    exit {code};;\n  esac\ndone\n");
            script += &format!("pwd -P > '{rec}/cwd'\n");
            for name in MARKERS {
                script += &format!("[ -n \"${{{name}+set}}\" ] && : > '{rec}/{name}'\n");
            }
            script += &format!(": > '{rec}/session'\nprintf '%s\\n' '{RESULT}'\nexit 0\n");
            let path = dir.join("claude");
            fs::write(&path, script).expect("write the stand-in");
            executable(&path);
            path
        };
        StandIn { path, records }
    }

    /// Whether a session started.
    pub fn started(&self) -> bool {
        self.records.join("session").exists()
    }

    /// The directory the session ran in.
    pub fn directory(&self) -> PathBuf {
        let recorded = fs::read_to_string(self.records.join("cwd")).expect("no session ran");
        plain(Path::new(recorded.trim()))
    }

    /// Whether the session got the variable `name`.
    pub fn got(&self, name: &str) -> bool {
        self.records.join(name).exists()
    }
}

/// What the stand-in's session prints: a result as Claude Code's JSON output ends with.
const RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"Done."}"#;

#[cfg(unix)]
fn executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("make it executable");
}

#[cfg(not(unix))]
fn executable(_: &Path) {}

/// A link at `link` to the file `target`.
pub fn symlink_file(target: &Path, link: &Path) {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    made.expect("make a symlink");
}

/// A local HTTP server that records the path of each request it gets and answers `{}`.
pub struct Witness {
    url: String,
    paths: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
}

impl Witness {
    pub fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind the witness");
        let url = format!("http://{}", listener.local_addr().expect("its address"));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let (recording, stopping) = (Arc::clone(&paths), Arc::clone(&stopped));
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let recording = Arc::clone(&recording);
                thread::spawn(move || witness(stream, &recording));
            }
        });
        Witness {
            url,
            paths,
            stopped,
        }
    }

    /// Its URL with `path`.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.url)
    }

    /// The paths of the requests it got, in order.
    pub fn paths(&self) -> Vec<String> {
        (self.paths.lock())
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl Drop for Witness {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
    }
}

/// Records the path of the request on `stream`, reads its body and answers `{}`.
fn witness(stream: TcpStream, paths: &Mutex<Vec<String>>) {
    let Ok(mut answer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_owned();
    (paths.lock())
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(path);
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let reply = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let _ = answer.write_all(reply.as_bytes());
}
