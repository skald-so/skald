//! A run as an agent whose output the test stops reading in the middle of a message with a long
//! text, and the local state's write lock a test takes so that a run that has written a message
//! waits to mark it.
//!
//! The message's text is far longer than a pipe holds, so a run whose output the test stops reading
//! waits in the middle of writing it. A run that has written it all waits to mark it while the test
//! holds the local state's write lock.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::cloud::WAIT;
use crate::kit::RECV;
use crate::machine::Machine;
use crate::world::AGENT;

/// How the message's text starts; nothing else holds it.
pub const HEAD: &str = "inventory export, every row of the warehouse:";

/// How long a run whose output is not read is given to fill the pipe.
pub const FILL: Duration = Duration::from_millis(500);

/// The message's text: [`HEAD`], then about 256 KiB, far more than a pipe holds.
pub fn long_text() -> String {
    format!("{HEAD} {}", "row of the export, ".repeat(13_500))
}

/// What a held run's reader and the test share.
#[derive(Default)]
struct Shared {
    /// What the test read of its output so far.
    output: Mutex<Vec<u8>>,
    /// Whether its output has ended.
    ended: AtomicBool,
    /// Whether reading stops at [`HEAD`].
    holding: AtomicBool,
}

/// A run as an agent whose output the test reads in small pieces, and stops reading once it holds
/// [`HEAD`] until let go.
pub struct Held {
    child: Child,
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
    errors: Option<JoinHandle<Vec<u8>>>,
}

impl Held {
    /// Starts `skald --agent <agent> recv` on `machine` in `dir`.
    pub fn start(machine: &Machine, dir: &Path, agent: &str) -> Self {
        Self::spawn(&mut machine.command_in(dir, &[AGENT, agent, RECV]))
    }

    /// Starts `command`, a run of `skald` as an agent.
    pub fn spawn(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run skald");
        let shared = Arc::new(Shared::default());
        shared.holding.store(true, Ordering::SeqCst);
        let stdout = child.stdout.take().expect("stdout is piped");
        let reader = read_holding(stdout, Arc::clone(&shared));
        let stderr: ChildStderr = child.stderr.take().expect("stderr is piped");
        let errors = thread::spawn(move || {
            let mut errors = Vec::new();
            let _ = { stderr }.read_to_end(&mut errors);
            errors
        });
        Held {
            child,
            shared,
            reader: Some(reader),
            errors: Some(errors),
        }
    }

    /// What it wrote to its output so far that the test read.
    pub fn shown(&self) -> String {
        let output = self.shared.output.lock().expect("a run's output");
        String::from_utf8_lossy(&output).into_owned()
    }

    /// Waits for up to [`WAIT`] until what the test read of its output holds `part`; fails if its
    /// output ends first.
    #[track_caller]
    pub fn wait_for(&mut self, part: &str) {
        let deadline = Instant::now() + WAIT;
        loop {
            if self.shown().contains(part) {
                return;
            }
            if self.shared.ended.load(Ordering::SeqCst) || Instant::now() > deadline {
                let what = &part[..part.len().min(HEAD.len())];
                let finished = self.kill();
                panic!("no {what:?} in the agent's output: {finished}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Reads the rest of its output from now on.
    pub fn let_go(&self) {
        self.shared.holding.store(false, Ordering::SeqCst);
    }

    /// Kills it; gives what it wrote to both outputs.
    pub fn kill(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.let_go();
        if let Some(reader) = self.reader.take() {
            reader.join().expect("read skald's output");
        }
        let errors = (self.errors.take())
            .map(|errors| errors.join().expect("read skald's errors"))
            .unwrap_or_default();
        format!("{}\n{}", self.shown(), String::from_utf8_lossy(&errors))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads `stdout` into the shared output in small pieces until it ends, then marks it ended; while
/// holding, stops once the output holds [`HEAD`].
fn read_holding(mut stdout: ChildStdout, shared: Arc<Shared>) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut piece = [0; 256];
        loop {
            let held = shared.holding.load(Ordering::SeqCst) && {
                let output = shared.output.lock().expect("a run's output");
                String::from_utf8_lossy(&output).contains(HEAD)
            };
            if held {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            match stdout.read(&mut piece) {
                Ok(0) | Err(_) => break,
                Ok(count) => (shared.output.lock())
                    .expect("a run's output")
                    .extend_from_slice(&piece[..count]),
            }
        }
        shared.ended.store(true, Ordering::SeqCst);
    })
}

/// Takes the write lock of `machine`'s local state; it holds until the connection is dropped or
/// rolls back.
pub fn write_lock(machine: &Machine) -> Connection {
    let state =
        Connection::open(machine.state_dir().join("state.sqlite3")).expect("open the local state");
    state
        .busy_timeout(Duration::from_secs(5))
        .expect("set a busy timeout");
    state
        .execute_batch("BEGIN IMMEDIATE")
        .expect("take the local state's write lock while a run writes its output");
    state
}
