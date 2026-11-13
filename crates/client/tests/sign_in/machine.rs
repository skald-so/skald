//! A machine without a browser: `skald` runs with an empty environment ([`bare_skald`]), a `PATH`
//! with nothing on it and a home, a state directory and a temporary directory of its own, all under
//! one directory that is removed on drop. Several machines run on one this way.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::common::CLIENT;

/// The setting of the directory of the local state and of the secrets file.
pub const STATE_DIR: &str = "SKALD_STATE_DIR";
/// The setting of the key storage; `file` keeps secrets in the 0600 file, never in the system's.
pub const KEY_STORAGE: &str = "SKALD_KEY_STORAGE";
/// The sign-in command; its flags of the server's address and of a `name` the user gives.
pub const SIGN_IN: &str = "sign-in";
pub const SERVER: &str = "--server";
pub const NAME: &str = "--name";
/// The flag of `skald sign-in` that answers whether this is the account's first device, and its
/// answers.
pub const FIRST_DEVICE: &str = "--first-device";
pub const YES: &str = "yes";
pub const NO: &str = "no";
/// The sign-out command.
pub const SIGN_OUT: &str = "sign-out";
/// The device list command; what it says on the current device's line, and above the devices
/// without a valid certificate and on a device without one.
pub const DEVICES: &str = "devices";
pub const THIS_DEVICE: &str = "this device";
pub const NOT_LINKED: &str = "not linked";
/// The setting of the host name the client reads, which names its device unless the user gives a
/// name; set to an empty value, the host name is empty.
pub const HOST_NAME: &str = "SKALD_HOST_NAME";

/// How long a run may take before the test fails.
pub const RUN_LIMIT: Duration = Duration::from_secs(90);

/// `skald` with an empty environment but for what Windows needs to run any program with sockets:
/// without `SystemRoot`, Winsock loads no provider, os error 10106.
pub fn bare_skald() -> Command {
    let mut command = Command::new(CLIENT);
    command.env_clear();
    #[cfg(windows)]
    if let Some(root) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", root);
    }
    command
}

pub struct Machine {
    root: PathBuf,
    /// The host name the client reads, if set; otherwise the system's.
    host_name: Option<String>,
}

impl Machine {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "skald-client-sign-in-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        for dir in ["home", "state", "tmp", "bin"] {
            fs::create_dir_all(root.join(dir)).expect("create a machine's directory");
        }
        Machine {
            root,
            host_name: None,
        }
    }

    /// The machine with the host name `name`.
    pub fn with_host_name(mut self, name: &str) -> Self {
        self.host_name = Some(name.to_owned());
        self
    }

    /// Everything the machine has on disk: its home, its state directory and its temporary files.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory its state and secrets file go to.
    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    /// Starts `skald sign-in --server <server>`.
    pub fn sign_in(&self, server: &str) -> Run {
        self.start(&[SIGN_IN, SERVER, server])
    }

    /// Starts `skald` with `args`.
    pub fn start(&self, args: &[&str]) -> Run {
        self.spawn(args, Stdio::null())
    }

    /// Starts `skald` with `args` and its input open, for [`Run::answer`].
    pub fn start_asking(&self, args: &[&str]) -> Run {
        self.spawn(args, Stdio::piped())
    }

    fn spawn(&self, args: &[&str], stdin: Stdio) -> Run {
        let mut command = bare_skald();
        command
            .args(args)
            .env("PATH", self.root.join("bin"))
            // The home and the temporary directory, as Unix and Windows name them.
            .env("HOME", self.root.join("home"))
            .env("USERPROFILE", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("TMP", self.root.join("tmp"))
            .env("TEMP", self.root.join("tmp"))
            .env(STATE_DIR, self.state_dir())
            .env(KEY_STORAGE, "file");
        if let Some(name) = &self.host_name {
            command.env(HOST_NAME, name);
        }
        let mut child = command
            .stdin(stdin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run skald");
        let input = child.stdin.take();
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let readers = vec![
            collect(child.stdout.take().expect("stdout is piped"), &stdout),
            collect(child.stderr.take().expect("stderr is piped"), &stderr),
        ];
        Run {
            child,
            input,
            started: Instant::now(),
            stdout,
            stderr,
            readers,
        }
    }

    /// Every file under [`Machine::root`].
    pub fn files(&self) -> Vec<PathBuf> {
        files(&self.root)
    }

    /// Every file under [`Machine::root`] whose bytes hold `secret`.
    pub fn files_holding(&self, secret: &str) -> Vec<PathBuf> {
        files(&self.root)
            .into_iter()
            .filter(|file| {
                fs::read(file).is_ok_and(|bytes| {
                    bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                })
            })
            .collect()
    }

    /// The bytes of its local state: its SQLite databases and their journals.
    pub fn state_bytes(&self) -> Vec<u8> {
        let files = self.sqlite_files();
        assert!(!files.is_empty(), "no local state in {:?}", self.root);
        files
            .iter()
            .flat_map(|file| fs::read(file).expect("read the local state"))
            .collect()
    }

    /// The files in the state directory that start as an SQLite database does, and their journals.
    pub fn sqlite_files(&self) -> Vec<PathBuf> {
        let databases: Vec<PathBuf> = files(&self.state_dir())
            .into_iter()
            .filter(|file| {
                fs::read(file).is_ok_and(|bytes| bytes.starts_with(b"SQLite format 3\0"))
            })
            .collect();
        let mut all = files(&self.state_dir());
        all.retain(|file| {
            databases.iter().any(|database| {
                let name = file.to_string_lossy();
                name.starts_with(&*database.to_string_lossy())
            })
        });
        all
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Every file under `dir`, at any depth.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => found.extend(files(&path)),
            Ok(kind) if kind.is_file() => found.push(path),
            _ => {}
        }
    }
    found
}

fn collect(mut from: impl Read + Send + 'static, into: &Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    let into = Arc::clone(into);
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        while let Ok(count) = from.read(&mut buffer) {
            if count == 0 {
                break;
            }
            into.lock()
                .expect("a run's output")
                .extend_from_slice(&buffer[..count]);
        }
    })
}

/// A running `skald`; killed on drop.
pub struct Run {
    child: Child,
    /// Its input, if open.
    input: Option<ChildStdin>,
    started: Instant,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    readers: Vec<JoinHandle<()>>,
}

/// What a finished run printed, and how it exited.
#[derive(Debug)]
pub struct Finished {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Finished {
    /// Both of its outputs: what a person at the terminal sees.
    pub fn shown(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }
}

impl Run {
    /// What it printed so far, stdout and stderr both.
    pub fn shown(&self) -> String {
        let stdout = self.stdout.lock().expect("a run's output");
        let stderr = self.stderr.lock().expect("a run's output");
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        )
    }

    /// Types `text` on its input, then closes it.
    pub fn answer(&mut self, text: &str) {
        let mut input = self.input.take().expect("a run started asking");
        input
            .write_all(text.as_bytes())
            .expect("type on skald's input");
    }

    /// Fails if it has exited.
    pub fn assert_running(&mut self) {
        if let Some(status) = self.child.try_wait().expect("wait for skald") {
            panic!("skald exited ({status}): {}", self.shown());
        }
    }

    /// Waits until what it printed holds each of `parts`, for up to [`RUN_LIMIT`].
    pub fn wait_to_show(&mut self, parts: &[&str]) {
        loop {
            let shown = self.shown();
            if parts.iter().all(|part| shown.contains(part)) {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("wait for skald") {
                panic!("skald exited ({status}) without showing {parts:?}: {shown}");
            }
            assert!(
                self.started.elapsed() < RUN_LIMIT,
                "skald did not show {parts:?}: {shown}"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits for it to exit, for up to [`RUN_LIMIT`] from its start.
    pub fn finish(mut self) -> Finished {
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for skald") {
                break status;
            }
            if self.started.elapsed() > RUN_LIMIT {
                let _ = self.child.kill();
                panic!("skald ran for more than {RUN_LIMIT:?}: {}", self.shown());
            }
            thread::sleep(Duration::from_millis(50));
        };
        for reader in self.readers.drain(..) {
            reader.join().expect("read skald's output");
        }
        let text = |output: &Arc<Mutex<Vec<u8>>>| {
            String::from_utf8(output.lock().expect("a run's output").clone())
                .expect("skald prints UTF-8")
        };
        Finished {
            success: status.success(),
            stdout: text(&self.stdout),
            stderr: text(&self.stderr),
        }
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
