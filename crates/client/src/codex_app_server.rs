//! The background app-server of a Codex config home as a path that tells that a registered Codex
//! session waits on Codex's own permission prompt (`docs/dev/agent-delivery.md`, "Codex permission
//! prompts"; [`crate::permission_prompts`]):
//!
//! - The helper keeps a client on the app-server of each config home that a live registered session
//!   runs in ([`Watchers`]): a WebSocket on the Unix socket that
//!   `app-server-control/app-server-control.sock` there links to. With none, it tries again every
//!   [`AGAIN`].
//! - The client sends `initialize`, then `initialized`; it lists the loaded threads every
//!   [`LIST_EVERY`] and subscribes with `thread/resume` to each registered session's thread among
//!   them, so it loads no thread itself ([`Client`]).
//! - A request `*/requestApproval`, or `thread/status/changed` with the flag `waitingOnApproval`,
//!   tells that the thread waits on a prompt; `serverRequest/resolved` of the thread, whichever
//!   answer the person gave in the terminal, ends it. So does a connection that ends meanwhile.
//! - It answers no request of the app-server: the person answers the prompt in the terminal.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tungstenite::Message;

use crate::permission_prompts;
use crate::state::{CodexSession, State};

/// Where a config home links the socket of its background app-server.
pub const CONTROL: [&str; 2] = ["app-server-control", "app-server-control.sock"];

/// How long a client waits before it connects again, once it has no connection.
const AGAIN: Duration = Duration::from_secs(1);

/// How often a client lists the loaded threads while a registered one is not subscribed to.
const LIST_EVERY: Duration = Duration::from_millis(300);

/// How long a read waits before the client looks at what else to do.
const TICK: Duration = Duration::from_millis(50);

/// How long the WebSocket's handshake may take.
const HANDSHAKE: Duration = Duration::from_secs(5);

/// The flag of a thread's status while it waits on approval.
const WAITING: &str = "waitingOnApproval";

/// The clients the helper keeps, by config home.
#[derive(Default)]
pub struct Watchers {
    by_home: BTreeMap<PathBuf, Watcher>,
}

impl Watchers {
    /// Keeps a client on the app-server of each config home of `sessions`, the live registered
    /// ones, by the state in `dir`, subscribed to their threads; ends the others.
    pub fn keep<'a>(&mut self, dir: &Path, sessions: impl IntoIterator<Item = &'a CodexSession>) {
        let mut homes: BTreeMap<PathBuf, BTreeMap<String, CodexSession>> = BTreeMap::new();
        for session in sessions {
            if let Some(home) = home_of(session) {
                let threads = homes.entry(home).or_default();
                threads.insert(session.thread.clone(), session.clone());
            }
        }
        self.by_home.retain(|home, _| homes.contains_key(home));
        for (home, threads) in homes {
            let watcher = (self.by_home.entry(home.clone()))
                .or_insert_with(|| Watcher::start(dir.to_owned(), home));
            *watcher
                .shared
                .threads
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = threads;
        }
    }
}

/// The config home of `session`: the one its hooks gave, or Codex's default, `~/.codex`.
fn home_of(session: &CodexSession) -> Option<PathBuf> {
    (session.home.clone())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))
}

/// The socket of the background app-server of the config home `home`: the one its link names, or
/// the link itself if it is no link.
fn socket_of(home: &Path) -> PathBuf {
    let link = home.join(CONTROL[0]).join(CONTROL[1]);
    match std::fs::read_link(&link) {
        Ok(target) => home.join(CONTROL[0]).join(target),
        Err(_) => link,
    }
}

/// A client on one config home's app-server, on a thread of its own, which ends on drop.
struct Watcher {
    shared: Arc<Shared>,
}

struct Shared {
    stopped: AtomicBool,
    /// The registered sessions of the config home, by thread.
    threads: Mutex<BTreeMap<String, CodexSession>>,
}

impl Shared {
    fn wanted(&self) -> BTreeSet<String> {
        let threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        threads.keys().cloned().collect()
    }

    fn session(&self, thread: &str) -> Option<CodexSession> {
        let threads = self.threads.lock().unwrap_or_else(PoisonError::into_inner);
        threads.get(thread).cloned()
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::SeqCst)
    }
}

impl Watcher {
    fn start(dir: PathBuf, home: PathBuf) -> Self {
        let shared = Arc::new(Shared {
            stopped: AtomicBool::new(false),
            threads: Mutex::new(BTreeMap::new()),
        });
        let watching = Arc::clone(&shared);
        thread::spawn(move || {
            while !watching.stopped() {
                if let Ok(stream) = UnixStream::connect(socket_of(&home)) {
                    watch(&dir, stream, &watching);
                }
                let since = Instant::now();
                while since.elapsed() < AGAIN && !watching.stopped() {
                    thread::sleep(TICK);
                }
            }
        });
        Watcher { shared }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::SeqCst);
    }
}

/// Serves the connection `stream` to an app-server until it ends or the client stops, noting each
/// prompt of a registered thread in the state in `dir`.
fn watch(dir: &Path, stream: UnixStream, shared: &Shared) {
    if stream.set_read_timeout(Some(HANDSHAKE)).is_err() {
        return;
    }
    let Ok((mut socket, _)) = tungstenite::client("ws://localhost/", stream) else {
        return;
    };
    if socket.get_ref().set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let mut client = Client::default();
    let mut sends = vec![client.start()];
    let mut listed: Option<Instant> = None;
    'serving: while !shared.stopped() {
        let wanted = shared.wanted();
        if listed.is_none_or(|listed| listed.elapsed() >= LIST_EVERY)
            && let Some(list) = client.list(&wanted)
        {
            sends.push(list);
            listed = Some(Instant::now());
        }
        for text in sends.drain(..) {
            if socket.send(Message::text(text)).is_err() {
                break 'serving;
            }
        }
        let text = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(_) => continue,
            Err(tungstenite::Error::Io(err))
                if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                continue;
            }
            Err(_) => break,
        };
        for effect in client.take(text.as_str(), &wanted) {
            match effect {
                Effect::Send(text) => sends.push(text),
                Effect::Told(thread) => {
                    if let Some(session) = shared.session(&thread)
                        && let Ok(state) = State::open(dir)
                    {
                        let _ = permission_prompts::codex_told(&state, &session);
                    }
                }
                Effect::Ended(thread) => {
                    let _ = permission_prompts::ended(dir, Some(&thread));
                }
            }
        }
    }
    let _ = socket.close(None);
    for thread in client.open {
        let _ = permission_prompts::ended(dir, Some(&thread));
    }
}

/// What a client does with a message of the app-server.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    /// Sends a message.
    Send(String),
    /// Notes that the thread waits on a prompt.
    Told(String),
    /// Notes that the thread's prompt ended.
    Ended(String),
}

/// A client's part of the app-server's protocol, apart from its connection.
#[derive(Debug, Default)]
struct Client {
    /// The ID of its next request.
    next: u64,
    initialized: bool,
    /// Its requests of the loaded threads, and of a subscription to each thread.
    lists: BTreeSet<u64>,
    resumes: BTreeMap<u64, String>,
    subscribed: BTreeSet<String>,
    /// The threads whose prompt it told and that have not ended.
    open: BTreeSet<String>,
}

impl Client {
    /// The request of `method` with `params`.
    fn request(&mut self, method: &str, params: Value) -> String {
        let id = self.next;
        self.next += 1;
        json!({ "id": id, "method": method, "params": params }).to_string()
    }

    /// The first request: `initialize`.
    fn start(&mut self) -> String {
        let client = json!({
            "name": "skald",
            "title": "Skald",
            "version": env!("CARGO_PKG_VERSION"),
        });
        self.request("initialize", json!({ "clientInfo": client }))
    }

    /// The request of the loaded threads, once initialized, while one of `wanted` is not
    /// subscribed to.
    fn list(&mut self, wanted: &BTreeSet<String>) -> Option<String> {
        if !self.initialized || wanted.is_subset(&self.subscribed) {
            return None;
        }
        let list = self.request("thread/loaded/list", json!({}));
        self.lists.insert(self.next - 1);
        Some(list)
    }

    /// What the client does with `text`, a message of the app-server, `wanted` the registered
    /// threads.
    fn take(&mut self, text: &str, wanted: &BTreeSet<String>) -> Vec<Effect> {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            return Vec::new();
        };
        let params = &message["params"];
        let thread = params["threadId"]
            .as_str()
            .filter(|thread| wanted.contains(*thread));
        let Some(method) = message["method"].as_str() else {
            return self.answered(&message, wanted);
        };
        let Some(thread) = thread else {
            return Vec::new();
        };
        let tells = match method {
            "thread/status/changed" => (params["status"]["activeFlags"]
                .as_array()
                .into_iter()
                .flatten())
            .any(|flag| flag == WAITING),
            "serverRequest/resolved" => {
                self.open.remove(thread);
                return vec![Effect::Ended(thread.to_owned())];
            }
            _ => message.get("id").is_some() && method.ends_with("/requestApproval"),
        };
        if !tells {
            return Vec::new();
        }
        self.open.insert(thread.to_owned());
        vec![Effect::Told(thread.to_owned())]
    }

    /// What the client does with `message`, the answer to one of its requests.
    fn answered(&mut self, message: &Value, wanted: &BTreeSet<String>) -> Vec<Effect> {
        let Some(id) = message["id"].as_u64() else {
            return Vec::new();
        };
        let failed = message.get("error").is_some_and(|error| !error.is_null());
        if id == 0 && !self.initialized {
            if failed {
                return Vec::new();
            }
            self.initialized = true;
            return vec![Effect::Send(json!({ "method": "initialized" }).to_string())];
        }
        if self.lists.remove(&id) {
            let loaded = message["result"]["data"].as_array().into_iter().flatten();
            let asked: Vec<String> = (loaded.filter_map(Value::as_str))
                .filter(|thread| wanted.contains(*thread) && !self.subscribed.contains(*thread))
                .filter(|thread| !self.resumes.values().any(|asked| asked == thread))
                .map(str::to_owned)
                .collect();
            return (asked.into_iter())
                .map(|thread| {
                    let params = json!({ "threadId": thread, "excludeTurns": true });
                    let resume = self.request("thread/resume", params);
                    self.resumes.insert(self.next - 1, thread);
                    Effect::Send(resume)
                })
                .collect();
        }
        if let Some(thread) = self.resumes.remove(&id)
            && !failed
        {
            self.subscribed.insert(thread);
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEB: &str = "019a-web";
    const API: &str = "019a-api";

    fn wanted() -> BTreeSet<String> {
        [WEB.to_owned()].into()
    }

    fn sent(effects: &[Effect]) -> Vec<Value> {
        (effects.iter())
            .filter_map(|effect| match effect {
                Effect::Send(text) => serde_json::from_str(text).ok(),
                _ => None,
            })
            .collect()
    }

    /// A client initialized and subscribed to web's thread.
    fn subscribed() -> Client {
        let mut client = Client::default();
        client.start();
        client.take(r#"{"id":0,"result":{}}"#, &wanted());
        client.list(&wanted()).unwrap();
        let loaded = json!({ "id": 1, "result": { "data": [WEB, API] } }).to_string();
        client.take(&loaded, &wanted());
        client.take(r#"{"id":2,"result":{"thread":{}}}"#, &wanted());
        client
    }

    /// The client initializes, lists the loaded threads and subscribes to the registered ones
    /// among them only; once subscribed to each, it lists no more.
    #[test]
    fn client_subscribes_to_registered_loaded_threads() {
        let mut client = Client::default();
        let start: Value = serde_json::from_str(&client.start()).unwrap();
        assert_eq!(start["method"], "initialize");
        assert_eq!(start["params"]["clientInfo"]["name"], "skald");
        assert_eq!(client.list(&wanted()), None, "before it is initialized");
        let initialized = client.take(r#"{"id":0,"result":{}}"#, &wanted());
        assert_eq!(sent(&initialized), [json!({ "method": "initialized" })]);
        let list: Value = serde_json::from_str(&client.list(&wanted()).unwrap()).unwrap();
        assert_eq!(list["method"], "thread/loaded/list");
        let loaded = json!({ "id": list["id"], "result": { "data": [WEB, API] } }).to_string();
        let resumes = sent(&client.take(&loaded, &wanted()));
        assert_eq!(resumes.len(), 1, "{resumes:?}");
        assert_eq!(resumes[0]["method"], "thread/resume");
        assert_eq!(resumes[0]["params"]["threadId"], WEB);
        assert!(sent(&client.take(&loaded, &wanted())).is_empty(), "asked");
        let failed = json!({ "id": resumes[0]["id"], "error": { "code": -32600 } }).to_string();
        client.take(&failed, &wanted());
        assert!(client.list(&wanted()).is_some(), "after a failed resume");
        let mut client = subscribed();
        assert_eq!(client.list(&wanted()), None, "subscribed");
    }

    /// A request for approval or a status with the flag tells a registered thread's prompt; its
    /// resolution ends it; a thread not registered, or a notification of the request, is none.
    #[test]
    fn approvals_tell_and_resolutions_end_a_registered_thread_s_prompt() {
        let mut client = subscribed();
        let request = |thread: &str| {
            json!({
                "id": 0,
                "method": "item/commandExecution/requestApproval",
                "params": { "threadId": thread, "command": "touch prompted.txt" },
            })
            .to_string()
        };
        let told = vec![Effect::Told(WEB.to_owned())];
        assert_eq!(client.take(&request(WEB), &wanted()), told);
        assert!(client.take(&request(API), &wanted()).is_empty());
        let status = |flags: Value| {
            json!({
                "method": "thread/status/changed",
                "params": { "threadId": WEB, "status": { "activeFlags": flags, "type": "active" } },
            })
            .to_string()
        };
        assert_eq!(client.take(&status(json!([WAITING])), &wanted()), told);
        assert!(client.take(&status(json!([])), &wanted()).is_empty());
        let notice = json!({
            "method": "item/commandExecution/requestApproval",
            "params": { "threadId": WEB },
        });
        assert!(client.take(&notice.to_string(), &wanted()).is_empty());
        assert_eq!(client.open, [WEB.to_owned()].into());
        let resolved = json!({
            "method": "serverRequest/resolved",
            "params": { "threadId": WEB, "requestId": 0 },
        });
        assert_eq!(
            client.take(&resolved.to_string(), &wanted()),
            [Effect::Ended(WEB.to_owned())]
        );
        assert!(client.open.is_empty());
    }

    /// The socket is the one the config home's link names, relative or whole; with no link, the
    /// link's own path.
    #[test]
    fn socket_is_the_one_the_config_home_links() {
        let dir = crate::test_dir::TestDir::new("codex-app-server-socket");
        let home = dir.join(".codex");
        let control = home.join(CONTROL[0]);
        std::fs::create_dir_all(&control).unwrap();
        assert_eq!(socket_of(&home), control.join(CONTROL[1]));
        std::os::unix::fs::symlink("/tmp/a.sock", control.join(CONTROL[1])).unwrap();
        assert_eq!(socket_of(&home), PathBuf::from("/tmp/a.sock"));
        std::fs::remove_file(control.join(CONTROL[1])).unwrap();
        std::os::unix::fs::symlink("b.sock", control.join(CONTROL[1])).unwrap();
        assert_eq!(socket_of(&home), control.join("b.sock"));
    }
}
