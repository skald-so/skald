//! A CLI killed during a handover: before its output is written, or after it but before the mark,
//! the next run as the agent hands the message over again; after the mark, nothing again.
//!
//! The message's text is far longer than a pipe holds, so a run whose output the test stops reading
//! waits in the middle of writing it. A run that has written it all waits to mark it while the test
//! holds the local state's write lock.

use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rusqlite::Connection;

use crate::cloud::WAIT;
use crate::kit::{RECV, received};
use crate::machine::Machine;
use crate::talk::{BOB_HOST, Line, assert_shows_once};
use crate::world::AGENT;
use crate::{Agents, CLAUDE, assert_handed_once, assert_not_handed};

/// How the message's text starts; nothing else holds it.
const HEAD: &str = "inventory export, every row of the warehouse:";

/// How long a run whose output is not read is given to fill the pipe.
const FILL: Duration = Duration::from_millis(500);

/// The message's text: [`HEAD`], then about 256 KiB, far more than a pipe holds.
fn long_text() -> String {
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
struct Held {
    child: Child,
    shared: Arc<Shared>,
    reader: Option<JoinHandle<()>>,
    errors: Option<JoinHandle<Vec<u8>>>,
}

impl Held {
    /// Starts `skald --agent <agent> recv` on `machine` in `dir`.
    fn start(machine: &Machine, dir: &Path, agent: &str) -> Self {
        let mut child = machine
            .command_in(dir, &[AGENT, agent, RECV])
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
    fn shown(&self) -> String {
        let output = self.shared.output.lock().expect("a run's output");
        String::from_utf8_lossy(&output).into_owned()
    }

    /// Waits for up to [`WAIT`] until what the test read of its output holds `part`; fails if its
    /// output ends first.
    #[track_caller]
    fn wait_for(&mut self, part: &str) {
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
    fn let_go(&self) {
        self.shared.holding.store(false, Ordering::SeqCst);
    }

    /// Kills it; gives what it wrote to both outputs.
    fn kill(&mut self) -> String {
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
fn write_lock(machine: &Machine) -> Connection {
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

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends a message with a long text
/// to the channel, which the person's `skald recv` shows. claude's run, killed while it waits in
/// the middle of writing the text, never wrote it whole: the next one writes it whole, and, killed
/// while it waits to mark it, the next one shows it once, with its `msg_id`; after that run,
/// nothing again.
#[test]
fn cli_killed_before_its_output_or_before_the_mark_hands_the_message_over_again() {
    let mut agents = Agents::new(
        [4_147_075_001, 4_147_075_002],
        ["ada-agent-crashes", "bob-sends-the-export"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let text = long_text();
    let id = agents.bob_sends(&Line::new(&bob, 1, &text));
    let person = received(&agents.talk.ada.machine).shown();
    let channel = agents.talk.channel;
    assert_shows_once(&person, &text, (channel, &bob, BOB_HOST));
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);

    let mut before_output = Held::start(machine, folder, CLAUDE);
    before_output.wait_for(HEAD);
    thread::sleep(FILL);
    let killed = before_output.kill();

    assert!(!killed.contains(&text), "the text written whole");

    let mut before_mark = Held::start(machine, folder, CLAUDE);
    before_mark.wait_for(HEAD);
    thread::sleep(FILL);
    let lock = write_lock(machine);
    before_mark.let_go();
    before_mark.wait_for(&text);
    before_mark.kill();
    lock.execute_batch("ROLLBACK")
        .expect("let go of the local state's write lock");

    let next = agents.run_as(CLAUDE).shown();

    assert_handed_once(&next, &text, id);

    let after_mark = agents.run_as(CLAUDE).shown();

    assert_not_handed(&after_mark, HEAD);
}
