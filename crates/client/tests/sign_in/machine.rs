//! A machine without a browser: `skald` runs with an empty environment, a `PATH` with nothing on it
//! and a home, a state directory and a temporary directory of its own, all under one directory
//! that is removed on drop. Several machines run on one this way.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::common::CLIENT;

/// The setting of the directory of the local state and of the secrets file.
pub const STATE_DIR: &str = "SKALD_STATE_DIR";
/// The setting of the key storage; `file` keeps secrets in the 0600 file, never in the system's.
pub const KEY_STORAGE: &str = "SKALD_KEY_STORAGE";
/// The sign-in command.
pub const SIGN_IN: &str = "sign-in";

/// How long a run may take before the test fails.
pub const RUN_LIMIT: Duration = Duration::from_secs(90);

pub struct Machine {
    root: PathBuf,
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
        Machine { root }
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
        self.start(&[SIGN_IN, "--server", server])
    }

    /// Starts `skald` with `args`.
    pub fn start(&self, args: &[&str]) -> Run {
        let mut child = Command::new(CLIENT)
            .args(args)
            .env_clear()
            .env("PATH", self.root.join("bin"))
            .env("HOME", self.root.join("home"))
            .env("TMPDIR", self.root.join("tmp"))
            .env(STATE_DIR, self.state_dir())
            .env(KEY_STORAGE, "file")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run skald");
        let stdout = Arc::new(Mutex::new(Vec::new()));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let readers = vec![
            collect(child.stdout.take().expect("stdout is piped"), &stdout),
            collect(child.stderr.take().expect("stderr is piped"), &stderr),
        ];
        Run {
            child,
            started: Instant::now(),
            stdout,
            stderr,
            readers,
        }
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
