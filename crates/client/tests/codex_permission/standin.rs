//! A stand-in for Codex's background app-server in a config home: a WebSocket on a Unix socket in
//! a new short directory, as Unix sockets need, linked from
//! `app-server-control/app-server-control.sock` in the config home, as Codex 0.162.1 links its
//! background app-server's socket in a directory of its own (`docs/dev/agent-delivery.md`, "Codex
//! permission prompts"). The link's path is longer than a Unix socket's may be, as a config home's
//! can be: a client connects to the socket it links to.
//!
//! It speaks JSON-RPC as Codex 0.162.1's app-server does to the requests a client of a session
//! sends: `initialize`, then `thread/resume`, `thread/read`, `thread/loaded/list` and `thread/list`
//! of the threads it hosts; any other request gets the error of an unknown method, and any but
//! `initialize` before it the error of a connection not initialized. Only a connection that resumed
//! a thread gets its events: on a prompt, the messages recorded from the real app-server for its
//! command that waits on approval, then on its answer those of the person's "Yes" or "No", each with
//! the thread's ID, folder and times. It keeps every answer a client gives to its requests.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::{self, UnixListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use tungstenite::Message;

use crate::cloud::WAIT;
use crate::codex::Thread;

/// Where the background app-server's socket is in its config home.
pub const CONTROL_DIR: &str = "app-server-control";
pub const CONTROL_SOCKET: &str = "app-server-control.sock";

/// The result of `initialize`, recorded, with the paths of its temporary directory.
const INITIALIZE: &str = include_str!("recorded/initialize.json");

/// The messages a client resumed on a thread got when the thread's command began to wait on
/// approval: the thread's status with `waitingOnApproval`, the command's item and the request.
const ASKED: &str = include_str!("recorded/approval-asked.jsonl");

/// The messages after the request once the person answered "Yes, proceed": the request resolved,
/// the status without the flag, the command's item `completed`, and the turn's end.
const ACCEPTED: &str = include_str!("recorded/approval-accepted.jsonl");

/// The messages after the request once the person answered "No": the request resolved, the
/// status without the flag, the command's item `declined`, and the turn's end.
const DECLINED: &str = include_str!("recorded/approval-declined.jsonl");

/// The threads, folders and turns of the recordings, and the recorded config home.
const RECORDED_THREADS: [&str; 2] = [
    "01a1fceb-da19-7203-8bb9-807cd2c5b8f6",
    "01a1fcec-419d-7810-b269-b17720bf30e1",
];
const RECORDED_DIRS: [&str; 2] = ["/tmp/skald-live/a/project", "/tmp/skald-live/b/project"];
const RECORDED_TURNS: [&str; 2] = [
    "01a1fceb-dd48-7c31-bdf9-ff53f83f0aa6",
    "01a1fcec-4443-7772-a9a2-a6d8cdfff339",
];
const RECORDED_HOME: &str = "/tmp/skald-live/a/home/.codex";

/// The request that a command waits on approval, and the notification that it was answered.
const REQUEST: &str = "item/commandExecution/requestApproval";
const RESOLVED: &str = "serverRequest/resolved";

/// The version of Codex that a thread names.
const CLI_VERSION: &str = "0.162.1";

/// How often a connection checks for what to send.
const TICK: Duration = Duration::from_millis(20);

/// The person's answer to the prompt in the terminal.
#[derive(Debug, Clone, Copy)]
pub enum Answer {
    Accepted,
    Declined,
}

/// A client's connection.
struct Connection {
    /// What goes to the client.
    out: Sender<String>,
    initialized: bool,
    /// The threads whose events it gets.
    resumed: HashSet<String>,
    /// The ID of its next request to the client.
    next_request: u64,
    /// The ID of the request of each thread's prompt still open.
    open: HashMap<String, u64>,
}

#[derive(Default)]
struct State {
    /// The threads it hosts, each with its folder.
    threads: Vec<(String, PathBuf)>,
    connections: HashMap<usize, Connection>,
    /// Every answer a client gave to one of its requests.
    answers: Vec<Value>,
}

struct Shared {
    home: PathBuf,
    state: Mutex<State>,
    changed: Condvar,
    stopped: AtomicBool,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        (self.state.lock()).unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The stand-in; its socket, its directory and the link go on drop.
pub struct AppServer {
    dir: PathBuf,
    link: PathBuf,
    shared: Arc<Shared>,
    listening: Option<JoinHandle<()>>,
}

impl AppServer {
    /// A stand-in listening in the config home `home`.
    pub fn new(home: &Path) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = PathBuf::from(format!(
            "/tmp/skx-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the app-server's directory");
        let socket = dir.join("a.sock");
        let listener = UnixListener::bind(&socket).expect("bind the app-server's socket");
        listener
            .set_nonblocking(true)
            .expect("a listener that does not block");
        let control = home.join(CONTROL_DIR);
        fs::create_dir_all(&control).expect("make the app-server's directory in the config home");
        let link = control.join(CONTROL_SOCKET);
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&socket, &link).expect("link the app-server's socket");
        let shared = Arc::new(Shared {
            home: home.to_owned(),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            stopped: AtomicBool::new(false),
        });
        let serving = Arc::clone(&shared);
        let listening = thread::spawn(move || {
            let mut next = 0;
            while !serving.stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let shared = Arc::clone(&serving);
                        let id = next;
                        next += 1;
                        thread::spawn(move || serve(stream, &shared, id));
                    }
                    Err(_) => thread::sleep(TICK),
                }
            }
        });
        AppServer {
            dir,
            link,
            shared,
            listening: Some(listening),
        }
    }

    /// Hosts `thread`, a session of the terminal UI that joined it.
    pub fn hosts(&self, thread: &Thread) {
        let mut state = self.shared.state();
        state.threads.push((thread.id.clone(), thread.dir.clone()));
    }

    /// Waits until a client has resumed `thread`, for up to [`WAIT`].
    #[track_caller]
    pub fn await_resumed(&self, thread: &Thread) {
        let deadline = Instant::now() + WAIT;
        let mut state = self.shared.state();
        loop {
            let resumed = (state.connections.values())
                .any(|connection| connection.resumed.contains(&thread.id));
            if resumed {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no client resumed the thread {} within {WAIT:?}",
                thread.id
            );
            state = (self.shared.changed.wait_timeout(state, left))
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// `thread`'s command waits on approval: each client that resumed it gets the recorded
    /// messages. Gives when they went.
    #[track_caller]
    pub fn asks(&self, thread: &Thread) -> Instant {
        let mut state = self.shared.state();
        let at = Instant::now();
        let mut told = 0;
        for connection in state.connections.values_mut() {
            if !connection.resumed.contains(&thread.id) {
                continue;
            }
            let request = connection.next_request;
            connection.next_request += 1;
            connection.open.insert(thread.id.clone(), request);
            for line in ASKED.lines() {
                let mut message = recorded(line, thread);
                if message["method"] == REQUEST {
                    message["id"] = json!(request);
                }
                let _ = connection.out.send(message.to_string());
            }
            told += 1;
        }
        assert!(told > 0, "no client resumed the thread {}", thread.id);
        at
    }

    /// The person answered `thread`'s prompt with `answer` in the terminal: each client told of it
    /// gets the recorded messages.
    pub fn resolves(&self, thread: &Thread, answer: Answer) {
        let lines = match answer {
            Answer::Accepted => ACCEPTED,
            Answer::Declined => DECLINED,
        };
        let mut state = self.shared.state();
        for connection in state.connections.values_mut() {
            let Some(request) = connection.open.remove(&thread.id) else {
                continue;
            };
            for line in lines.lines() {
                let mut message = recorded(line, thread);
                if message["method"] == RESOLVED {
                    message["params"]["requestId"] = json!(request);
                }
                let _ = connection.out.send(message.to_string());
            }
        }
    }

    /// Every answer a client gave to one of its requests.
    pub fn answers(&self) -> Vec<Value> {
        self.shared.state().answers.clone()
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
        if let Some(listening) = self.listening.take() {
            let _ = listening.join();
        }
        let _ = fs::remove_file(&self.link);
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Now, in milliseconds since the Unix epoch.
fn now_ms() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    let millis = since.expect("after 1970").as_millis();
    u64::try_from(millis).expect("a time in milliseconds")
}

/// The recorded message `line` as `thread`'s, now: the recorded threads its ID, the recorded
/// folders its folder, the recorded turns one, and its times now.
fn recorded(line: &str, thread: &Thread) -> Value {
    let mut message: Value = serde_json::from_str(line).expect("a recorded message is JSON");
    as_now(&mut message, thread, now_ms());
    message
}

fn as_now(value: &mut Value, thread: &Thread, now: u64) {
    match value {
        Value::String(text) => {
            if RECORDED_THREADS.contains(&text.as_str()) {
                *text = thread.id.clone();
            } else if RECORDED_DIRS.contains(&text.as_str()) {
                *text = thread.dir.to_string_lossy().into_owned();
            } else if text == RECORDED_TURNS[1] {
                *text = RECORDED_TURNS[0].to_owned();
            }
        }
        Value::Array(items) => {
            for item in items {
                as_now(item, thread, now);
            }
        }
        Value::Object(fields) => {
            for (name, field) in fields.iter_mut() {
                if field.is_number() && name.ends_with("AtMs") {
                    *field = json!(now);
                } else if field.is_number() && (name == "startedAt" || name == "completedAt") {
                    *field = json!(now / 1_000);
                } else {
                    as_now(field, thread, now);
                }
            }
        }
        _ => {}
    }
}

/// A thread's object in a result.
fn thread_object(id: &str, dir: &Path) -> Value {
    json!({
        "id": id,
        "cwd": dir.to_string_lossy(),
        "ephemeral": false,
        "status": { "activeFlags": [], "type": "active" },
        "cliVersion": CLI_VERSION,
    })
}

/// Serves the client of `stream`, the connection `id`, until it closes it or the stand-in stops.
fn serve(stream: net::UnixStream, shared: &Shared, id: usize) {
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let Ok(mut socket) = tungstenite::accept(stream) else {
        return;
    };
    if socket.get_ref().set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let (out, queued) = mpsc::channel();
    shared.state().connections.insert(
        id,
        Connection {
            out,
            initialized: false,
            resumed: HashSet::new(),
            next_request: 0,
            open: HashMap::new(),
        },
    );
    'serving: while !shared.stopped.load(Ordering::SeqCst) {
        while let Ok(text) = queued.try_recv() {
            if socket.send(Message::text(text)).is_err() {
                break 'serving;
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if let Some(reply) = take(shared, id, text.as_str())
                    && socket.send(Message::text(reply)).is_err()
                {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    let _ = socket.close(None);
    shared.state().connections.remove(&id);
    shared.changed.notify_all();
}

/// Takes `text`, what the client of the connection `id` sent; gives the reply to a request.
fn take(shared: &Shared, id: usize, text: &str) -> Option<String> {
    let message: Value = serde_json::from_str(text).ok()?;
    let mut state = shared.state();
    let Some(method) = message["method"].as_str() else {
        if message.get("id").is_some() {
            state.answers.push(message);
            shared.changed.notify_all();
        }
        return None;
    };
    // A notification, such as `initialized`.
    let request = message.get("id")?.clone();
    let threads = state.threads.clone();
    let named = message["params"]["threadId"].as_str().unwrap_or_default();
    let hosted = threads.iter().find(|(thread, _)| thread == named);
    let connection = state.connections.get_mut(&id)?;
    let reply = match method {
        "initialize" => {
            connection.initialized = true;
            let home = shared.home.to_string_lossy();
            let result = INITIALIZE.trim().replace(RECORDED_HOME, &home);
            Ok(serde_json::from_str(&result).expect("a recorded result is JSON"))
        }
        _ if !connection.initialized => Err((-32_600, "Not initialized".to_owned())),
        "thread/resume" | "thread/read" => match hosted {
            Some((thread, dir)) => {
                if method == "thread/resume" {
                    connection.resumed.insert(thread.clone());
                }
                Ok(json!({ "thread": thread_object(thread, dir) }))
            }
            None => Err((-32_600, format!("no rollout found for thread id {named}"))),
        },
        "thread/loaded/list" => {
            let ids: Vec<&str> = threads.iter().map(|(thread, _)| thread.as_str()).collect();
            Ok(json!({ "data": ids, "nextCursor": null }))
        }
        "thread/list" => {
            let listed: Vec<Value> = (threads.iter())
                .map(|(thread, dir)| thread_object(thread, dir))
                .collect();
            Ok(json!({ "data": listed, "nextCursor": null }))
        }
        _ => Err((-32_601, format!("method not found: {method}"))),
    };
    shared.changed.notify_all();
    let reply = match reply {
        Ok(result) => json!({ "id": request, "result": result }),
        Err((code, text)) => json!({ "id": request, "error": { "code": code, "message": text } }),
    };
    Some(reply.to_string())
}
