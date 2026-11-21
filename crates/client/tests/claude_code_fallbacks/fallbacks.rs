//! What the tests share: Ada's machine with no session of an agent running, the copy of `skald`
//! that Claude Code's MCP server and hooks run without the one-shot setting, the channel events of
//! a `skald mcp`, and runs of the `asyncRewake` hook in the background.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::{sync::Arc, sync::Mutex, thread, time::Duration, time::Instant};

use serde_json::{Value, json};

use crate::client::{CLIENT, MCP, Session};
use crate::helped::Helped;
use crate::hooks::{CLAUDE_CODE, HOOK, Hooked};
use crate::kit::RECV;
use crate::machine::{Machine, RUN_LIMIT};
use crate::solo::{Solo, WEB};
use crate::world::AGENT;

/// The setting of how long a handover waits for its confirmation, in seconds.
pub const RESEND_SETTING: &str = "SKALD_RESEND_AFTER";

/// The flag that makes `skald hook claude` the `asyncRewake` hook, and how long it waits.
pub const WAIT_FLAG: &str = "--wait";

/// The exit code with which the `asyncRewake` hook starts a turn.
pub const REWAKES: i32 = 2;

/// The capability of a channel server, under `experimental`, and its events' notification.
pub const CHANNEL_CAPABILITY: &str = "claude/channel";
pub const CHANNEL_EVENT: &str = "notifications/claude/channel";

/// How long a test watches for what must not happen.
pub const QUIET: Duration = Duration::from_secs(5);

/// A `SessionStart` input recorded from Claude Code 2.1.329, with the paths of a temporary
/// directory.
const SESSION_START: &str = include_str!("../claude_code_socket/recorded/session-start.json");

/// Ada's machine alone in `clienta`, with no session of an agent running, and the copy of `skald`
/// that Claude Code runs. Her agent web has run once, so messages to it are its.
pub struct Bare {
    /// Kills the helpers before the machine's directory goes.
    pub helped: Helped,
    pub solo: Solo,
}

impl Bare {
    /// Ada, GitHub user `id` with `login`.
    pub fn new(id: u64, login: &str) -> Self {
        let solo = Solo::new(id, login);
        let helped = Helped::new(solo.machine());
        let ran = (solo.machine().start_in(&solo.dir(WEB), &[AGENT, WEB, RECV])).finish();
        assert!(ran.success, "web's skald recv: {ran:?}");
        Bare { helped, solo }
    }

    pub fn machine(&self) -> &Machine {
        self.solo.machine()
    }

    /// `skald --agent <agent> mcp` in the agent's folder, with `settings`, as Claude Code starts
    /// it, initialized as Claude Code.
    pub fn channel(&self, agent: &str, settings: &[(&str, &str)]) -> Session {
        channel_of(&self.helped, &self.solo.dir(agent), agent, settings)
    }

    /// The `asyncRewake` hook of `agent`, `--wait <seconds>`, on `input`, with `settings`.
    pub fn rewake(
        &self,
        agent: &str,
        input: &Value,
        seconds: u64,
        settings: &[(&str, &str)],
    ) -> Rewake {
        rewake_of(
            &self.helped,
            &self.solo.dir(agent),
            agent,
            input,
            seconds,
            settings,
        )
    }
}

/// `skald --agent <agent> mcp` of `helped` in `dir`, with `settings`, initialized as Claude Code.
pub fn channel_of(helped: &Helped, dir: &Path, agent: &str, settings: &[(&str, &str)]) -> Session {
    Session::start(
        &mut helped.command_in(dir, settings, &[AGENT, agent, MCP]),
        CLIENT,
    )
}

/// The `asyncRewake` hook of `agent` of `helped` in `dir`, `--wait <seconds>`, on `input`, its
/// `cwd` `dir`, with `settings`.
pub fn rewake_of(
    helped: &Helped,
    dir: &Path,
    agent: &str,
    input: &Value,
    seconds: u64,
    settings: &[(&str, &str)],
) -> Rewake {
    let seconds = seconds.to_string();
    let args = [AGENT, agent, HOOK, CLAUDE_CODE, WAIT_FLAG, seconds.as_str()];
    let mut input = input.clone();
    input["cwd"] = json!(dir.to_string_lossy());
    Rewake::start(
        &mut helped.command_in(dir, settings, &args),
        &input.to_string(),
    )
}

/// The recorded `SessionStart` input.
pub fn session_start() -> Value {
    serde_json::from_str(SESSION_START).expect("a recorded input is JSON")
}

/// The texts of the channel events `session` sent so far, waiting for up to `wait` until it sent
/// `count`.
#[track_caller]
pub fn events(session: &mut Session, count: usize, wait: Duration) -> Vec<String> {
    (session.notifications(CHANNEL_EVENT, count, wait).iter())
        .map(|event| match &event["params"]["content"] {
            Value::String(text) => text.clone(),
            other => panic!("an event's content is no text: {event}: {other}"),
        })
        .collect()
}

/// Waits for up to `wait` until `session` sent its `count`th channel event; fails if it did not.
/// Gives the events' texts.
#[track_caller]
pub fn wait_events(session: &mut Session, count: usize, wait: Duration) -> Vec<String> {
    let sent = events(session, count, wait);
    assert!(
        sent.len() >= count,
        "{} channel events, not {count}: {sent:#?}\n{}",
        sent.len(),
        session.errors()
    );
    sent
}

/// A text of `length` characters that starts with `head`, on one line.
pub fn long_text(head: &str, length: usize) -> String {
    let mut text = String::from(head);
    let mut row = 0;
    while text.chars().count() < length {
        row += 1;
        text.push_str(&format!(" row {row} of the report;"));
    }
    text.chars().take(length).collect()
}

/// A run of the `asyncRewake` hook in the background; killed on drop.
pub struct Rewake {
    child: Child,
    started: Instant,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// The threads that read its outputs.
    readers: Vec<thread::JoinHandle<()>>,
    /// When it exited, once it did, and with which code.
    exited: Option<(Instant, Option<i32>)>,
}

impl Rewake {
    /// Starts `command` with `input` on its standard input, which then closes.
    pub fn start(command: &mut Command, input: &str) -> Self {
        let spawned = (command.stdout(Stdio::piped()).stderr(Stdio::piped()))
            .stdin(Stdio::piped())
            .spawn();
        let mut child = spawned.expect("start the asyncRewake hook");
        if let Some(mut stdin) = child.stdin.take() {
            // A run that fails before reading its input has closed it: not the test's failure.
            let _ = stdin.write_all(input.as_bytes());
        }
        let (stdout, out) = gathered(child.stdout.take().expect("stdout is piped"));
        let (stderr, err) = gathered(child.stderr.take().expect("stderr is piped"));
        Rewake {
            child,
            started: Instant::now(),
            stdout,
            stderr,
            readers: vec![out, err],
            exited: None,
        }
    }

    /// Whether it still runs.
    pub fn running(&mut self) -> bool {
        if self.exited.is_none()
            && let Some(status) = self.child.try_wait().expect("wait for skald hook")
        {
            self.exited = Some((Instant::now(), status.code()));
        }
        self.exited.is_none()
    }

    /// Fails unless it still runs, saying `when`.
    #[track_caller]
    pub fn assert_running(&mut self, when: &str) {
        if !self.running() {
            panic!("{when}: the asyncRewake hook ended: {:?}", self.hooked());
        }
    }

    /// Waits for it to exit, for up to [`RUN_LIMIT`] from its start; fails if it does not. Gives
    /// what it printed, its exit code, and how long it ran.
    #[track_caller]
    pub fn finished(&mut self) -> (Hooked, Duration) {
        while self.running() {
            assert!(
                self.started.elapsed() < RUN_LIMIT,
                "the asyncRewake hook ran for more than {RUN_LIMIT:?}"
            );
            thread::sleep(Duration::from_millis(50));
        }
        let (at, _) = self.exited.expect("it exited");
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        (self.hooked(), at - self.started)
    }

    /// What it printed so far, and its exit code if it exited.
    fn hooked(&self) -> Hooked {
        let text = |bytes: &Arc<Mutex<Vec<u8>>>| {
            let bytes = bytes
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            String::from_utf8_lossy(&bytes).into_owned()
        };
        Hooked {
            code: self.exited.and_then(|(_, code)| code),
            stdout: text(&self.stdout),
            stderr: text(&self.stderr),
        }
    }
}

impl Drop for Rewake {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Reads `from` to its end on a thread of its own, into what it gives, with the thread.
fn gathered(mut from: impl Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, thread::JoinHandle<()>) {
    let into = Arc::new(Mutex::new(Vec::new()));
    let filling = Arc::clone(&into);
    let reading = thread::spawn(move || {
        let mut all = Vec::new();
        let _ = from.read_to_end(&mut all);
        (filling
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()))
        .extend(all);
    });
    (into, reading)
}

/// Fails unless `hooked`, a run of the `asyncRewake` hook, exited with [`REWAKES`]; gives what it
/// printed on its standard error.
#[track_caller]
pub fn woke(hooked: &Hooked) -> String {
    assert_eq!(
        hooked.code,
        Some(REWAKES),
        "the hook did not start a turn: {hooked:?}"
    );
    assert!(
        !hooked.stderr.trim().is_empty(),
        "it gave nothing: {hooked:?}"
    );
    hooked.stderr.clone()
}
