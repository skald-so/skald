//! A stand-in of a Claude Code session: its cross-session inbox socket and its session file, as
//! Claude Code 2.1.329 handles them (`docs/dev/agent-delivery.md`, "Waking an idle session", "Read
//! confirmations"), from what it recorded (`recorded/`).
//!
//! - The socket takes lines of JSON. An auth line, `{"type":"auth","token":<token>}`, with the
//!   session's token makes the later lines of its connection the session's own child's. A user
//!   line, `{"type":"user","message":{"role":"user","content":<text>},"from":<from>,
//!   "msg_id":<msg_id>,"session_id":<session>}`, is a message; the session drops one whose
//!   `session_id` is not its own.
//! - What the session does with a message is the test's choice ([`Take`]): take it into the
//!   conversation, as an idle session does at once; queue it, as a busy session does until its
//!   tool call ends; or answer it with a receipt, as a session that holds, refuses or drops it
//!   does. A receipt, `{"type":"control","action":"peer_message_status","status":<status>,
//!   "orig_msg_id":<msg_id>}`, goes to the line's `from` when it is a reply address,
//!   `uds:<path>`.
//! - The session file gets what Claude Code writes: a message the session queues as a
//!   `queue-operation`, one it takes into the conversation as a `user` entry whose `origin` holds
//!   the line's `from` and `msg_id`. The test adds the model's own entries and the other entries
//!   a session writes.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

use crate::cloud::WAIT;

/// The settings Claude Code gives its hooks, but `SessionEnd`: the session's socket and its token.
pub const SOCKET_VAR: &str = "CLAUDE_CODE_MESSAGING_SOCKET";
pub const TOKEN_VAR: &str = "CLAUDE_CODE_MESSAGING_TOKEN";

/// The statuses of the receipts that say a message did not reach the model.
pub const FAILING: [&str; 5] = ["held", "expired", "refused", "dropped", "denied"];
/// The status of the receipt of a held message the session released.
pub const RELEASED: &str = "delivered";

/// Entries recorded from Claude Code 2.1.329, their paths those of a temporary directory.
const PEER_MESSAGE: &str = include_str!("recorded/peer-message.json");
const QUEUED: &str = include_str!("recorded/queued.json");
const REPLY: &str = include_str!("recorded/reply.json");
const REASONING: &str = include_str!("recorded/reasoning.json");
const TOOL_CALL: &str = include_str!("recorded/tool-call.json");
const TOOL_RESULT: &str = include_str!("recorded/tool-result.json");
const HOOK_OUTPUT: &str = include_str!("recorded/hook-output.json");
const API_ERROR: &str = include_str!("recorded/api-error.json");

/// What the recorded message said; Claude Code wraps a message's text in what comes before and
/// after it.
const RECORDED_TEXT: &str = "[skald] own line MARKER-ONE";

/// What the stand-in does with the messages that come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// Takes each into the conversation.
    Accept,
    /// Queues each, as a busy session does, until [`Standin::dequeue`].
    Queue,
    /// Answers each with a receipt of this status and takes none.
    Receipt(&'static str),
}

/// A user line the socket got.
#[derive(Debug, Clone)]
pub struct Line {
    pub value: Value,
    /// Whether an auth line with the session's token came first on its connection.
    pub authed: bool,
    /// Whether it was its connection's first line after the auth line, if any.
    pub first: bool,
}

impl Line {
    /// Its text, as the session takes it.
    pub fn content(&self) -> String {
        match &self.value["message"]["content"] {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }
    }

    pub fn msg_id(&self) -> String {
        self.value["msg_id"].as_str().unwrap_or_default().to_owned()
    }

    fn from(&self) -> String {
        self.value["from"].as_str().unwrap_or_default().to_owned()
    }
}

#[derive(Default)]
struct Got {
    lines: Vec<Line>,
    /// The lines queued and not yet in the conversation, and those held.
    queued: Vec<Line>,
    held: Vec<Line>,
    /// Receipts sent, by status.
    receipts: Vec<(String, String)>,
    /// Lines whose receipt had no reply address to go to.
    unanswered: usize,
    /// The `uuid` of the session file's last conversation entry.
    last: Option<String>,
}

struct Shared {
    session_id: String,
    token: String,
    file: PathBuf,
    take: Mutex<Take>,
    got: Mutex<Got>,
    changed: Condvar,
    stopped: AtomicBool,
    /// How many connections the socket took.
    connections: AtomicUsize,
}

impl Shared {
    fn got(&self) -> MutexGuard<'_, Got> {
        self.got
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A Claude Code session's socket and file; the socket goes, and its directory, on drop.
pub struct Standin {
    dir: PathBuf,
    socket: PathBuf,
    shared: Arc<Shared>,
    /// The thread that holds the socket's listener.
    listening: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Standin {
    /// A session with a socket of its own in a new short directory, as Unix sockets need, and its
    /// session file there; it takes messages as `take` says.
    pub fn new(take: Take) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = PathBuf::from(format!(
            "/tmp/skc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the session's directory");
        let session_id = random_uuid();
        let socket = dir.join("s.sock");
        let listener = UnixListener::bind(&socket).expect("bind the session's socket");
        let shared = Arc::new(Shared {
            file: dir.join(format!("{session_id}.jsonl")),
            session_id,
            token: hex(&random::<24>()),
            take: Mutex::new(take),
            got: Mutex::new(Got::default()),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
        });
        let serving = Arc::clone(&shared);
        let listening = thread::spawn(move || {
            for stream in listener.incoming() {
                if serving.stopped.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                serving.connections.fetch_add(1, Ordering::SeqCst);
                let shared = Arc::clone(&serving);
                thread::spawn(move || read_connection(stream, &shared));
            }
        });
        Standin {
            dir,
            socket,
            shared,
            listening: Mutex::new(Some(listening)),
        }
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    pub fn token(&self) -> &str {
        &self.shared.token
    }

    pub fn session_id(&self) -> &str {
        &self.shared.session_id
    }

    /// The session file, as Claude Code names it in hook inputs.
    pub fn file(&self) -> &Path {
        &self.shared.file
    }

    /// From now on, takes messages as `take` says.
    pub fn take(&self, take: Take) {
        *self
            .shared
            .take
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = take;
    }

    /// The user lines the socket got so far, in order.
    pub fn lines(&self) -> Vec<Line> {
        self.shared.got().lines.clone()
    }

    /// The user lines whose content holds `text`.
    pub fn lines_holding(&self, text: &str) -> Vec<Line> {
        (self.lines().into_iter())
            .filter(|line| line.content().contains(text))
            .collect()
    }

    /// Waits for up to [`WAIT`] until the socket got `count` user lines in all; fails if it did
    /// not. Gives them.
    #[track_caller]
    pub fn wait_lines(&self, count: usize) -> Vec<Line> {
        let deadline = Instant::now() + WAIT;
        let mut got = self.shared.got();
        while got.lines.len() < count {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "the session's socket got {} lines, not {count}: {:?}",
                got.lines.len(),
                got.lines
            );
            got = (self.shared.changed.wait_timeout(got, left))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
        got.lines.clone()
    }

    /// Waits for up to [`WAIT`] until the socket takes another connection, such as the helper's
    /// check that the session runs; fails if it does not.
    #[track_caller]
    pub fn wait_connection(&self) {
        let deadline = Instant::now() + WAIT;
        let taken = self.shared.connections.load(Ordering::SeqCst);
        while self.shared.connections.load(Ordering::SeqCst) == taken {
            assert!(
                Instant::now() < deadline,
                "the session's socket took no connection"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits for up to [`WAIT`] until the socket sent `count` receipts in all; fails if it did not
    /// or if a line had no reply address to send its receipt to. Gives them as `(status, msg_id)`.
    #[track_caller]
    pub fn wait_receipts(&self, count: usize) -> Vec<(String, String)> {
        let deadline = Instant::now() + WAIT;
        loop {
            let got = self.shared.got();
            assert_eq!(
                got.unanswered, 0,
                "a line had no reply address, uds:<path>, for its receipt: {:?}",
                got.lines
            );
            if got.receipts.len() >= count {
                return got.receipts.clone();
            }
            drop(got);
            assert!(
                Instant::now() < deadline,
                "the session sent fewer than {count} receipts"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Takes the queued messages into the conversation, as a busy session does once its tool call
    /// ends.
    pub fn dequeue(&self) {
        let queued = std::mem::take(&mut self.shared.got().queued);
        for line in &queued {
            converse(&self.shared, line);
        }
    }

    /// Releases the held messages: answers each with [`RELEASED`] and takes it into the
    /// conversation.
    pub fn release(&self) {
        let held = std::mem::take(&mut self.shared.got().held);
        for line in &held {
            answer(&self.shared, line, RELEASED);
            converse(&self.shared, line);
        }
    }

    /// The model replies with text.
    pub fn model_replies(&self) {
        self.append(REPLY, |_| {});
    }

    /// The model reasons.
    pub fn model_reasons(&self) {
        self.append(REASONING, |_| {});
    }

    /// The model calls a tool.
    pub fn model_calls_a_tool(&self) {
        self.append(TOOL_CALL, |_| {});
    }

    /// The call to the model fails: Claude Code writes an API error in the model's place.
    pub fn api_error(&self) {
        self.append(API_ERROR, |_| {});
    }

    /// A tool's result that holds `text` goes to the model.
    pub fn tool_result(&self, text: &str) {
        self.append(TOOL_RESULT, |entry| {
            entry["message"]["content"][0]["content"] = json!(text);
            entry["toolUseResult"]["stdout"] = json!(text);
        });
    }

    /// A hook's own output that holds `text`.
    pub fn hook_output(&self, text: &str) {
        self.append(HOOK_OUTPUT, |entry| {
            entry["attachment"]["content"] = json!(text);
            entry["attachment"]["stdout"] = json!(format!("{text}\n"));
        });
    }

    /// The socket goes, as when its session exits without `SessionEnd`: it takes no connection.
    pub fn gone(&self) {
        self.dies();
        let _ = fs::remove_file(&self.socket);
    }

    /// The session is killed without `SessionEnd`: its socket's file stays, and the socket takes
    /// no connection.
    pub fn dies(&self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        let listening = (self.listening.lock())
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(listening) = listening {
            // The connection wakes the listener, which then closes.
            let _ = UnixStream::connect(&self.socket);
            let _ = listening.join();
        }
    }

    /// Appends the recorded entry `recorded` as the session's, after its last, with `edit`.
    fn append(&self, recorded: &str, edit: impl FnOnce(&mut Value)) {
        write_entry(&self.shared, recorded, edit);
    }
}

impl Drop for Standin {
    fn drop(&mut self) {
        self.gone();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Reads the lines of one connection until it closes, and acts on each user line.
fn read_connection(stream: UnixStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(WAIT));
    let mut authed = false;
    let mut first = true;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["type"] == "auth" {
            authed = value["token"].as_str() == Some(shared.token.as_str());
            continue;
        }
        if value["type"] != "user" {
            continue;
        }
        let got = Line {
            value,
            authed,
            first,
        };
        first = false;
        let ours = got.value["session_id"].as_str() == Some(shared.session_id.as_str());
        if ours {
            take_line(shared, &got);
        }
        // Once the session acted on it, as a test that waits for the line acts after.
        shared.got().lines.push(got);
        shared.changed.notify_all();
    }
}

/// Acts on `got`, a user line of the session's own, as the session takes lines now.
fn take_line(shared: &Shared, got: &Line) {
    let take = *shared
        .take
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match take {
        Take::Accept => {
            queue(shared, got);
            converse(shared, got);
        }
        Take::Queue => {
            queue(shared, got);
            shared.got().queued.push(got.clone());
        }
        Take::Receipt(status) => {
            answer(shared, got, status);
            if status == "held" {
                shared.got().held.push(got.clone());
            }
        }
    }
}

/// Writes that the session queued `line`'s message.
fn queue(shared: &Shared, line: &Line) {
    let content = line.content();
    let mut entry: Value = serde_json::from_str(QUEUED).expect("a recorded entry is JSON");
    entry["sessionId"] = json!(shared.session_id);
    entry["content"] = json!(content);
    append_line(shared, &entry);
}

/// Writes `line`'s message into the conversation, as Claude Code wraps it.
fn converse(shared: &Shared, line: &Line) {
    let recorded: Value = serde_json::from_str(PEER_MESSAGE).expect("a recorded entry is JSON");
    let wrapped = recorded["message"]["content"].as_str().unwrap_or_default();
    let (before, after) = wrapped
        .split_once(RECORDED_TEXT)
        .expect("the recorded message holds its text");
    let content = format!("{before}{}{after}", line.content());
    let (from, msg_id) = (line.from(), line.msg_id());
    write_entry(shared, PEER_MESSAGE, |entry| {
        entry["message"]["content"] = json!(content);
        entry["origin"]["from"] = json!(from);
        entry["origin"]["msg_id"] = json!(msg_id);
    });
}

/// Sends `line`'s `from` a receipt of `status`, if it is a reply address.
fn answer(shared: &Shared, line: &Line, status: &str) {
    let from = line.from();
    let Some(path) = from.strip_prefix("uds:") else {
        shared.got().unanswered += 1;
        return;
    };
    let receipt = json!({
        "type": "control", "action": "peer_message_status", "status": status,
        "reason": "the stand-in session's answer", "orig_msg_id": line.msg_id(),
    });
    if let Ok(mut stream) = UnixStream::connect(path) {
        let _ = writeln!(stream, "{receipt}");
        let _ = stream.flush();
    }
    shared
        .got()
        .receipts
        .push((status.to_owned(), line.msg_id()));
}

/// Appends the recorded entry `recorded` with `edit`, as the session's, after its last.
fn write_entry(shared: &Shared, recorded: &str, edit: impl FnOnce(&mut Value)) {
    let mut entry: Value = serde_json::from_str(recorded).expect("a recorded entry is JSON");
    let uuid = random_uuid();
    let parent = shared.got().last.replace(uuid.clone());
    entry["uuid"] = json!(uuid);
    entry["parentUuid"] = json!(parent);
    entry["sessionId"] = json!(shared.session_id);
    edit(&mut entry);
    append_line(shared, &entry);
}

fn append_line(shared: &Shared, entry: &Value) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&shared.file)
        .expect("open the session file");
    // In one write, as no other writer's entry goes into it.
    (file.write_all(format!("{entry}\n").as_bytes())).expect("write the session file");
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).expect("random bytes");
    bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A random UUID, as Claude Code names sessions and entries.
fn random_uuid() -> String {
    let text = hex(&random::<16>());
    format!(
        "{}-{}-{}-{}-{}",
        &text[..8],
        &text[8..12],
        &text[12..16],
        &text[16..20],
        &text[20..]
    )
}
