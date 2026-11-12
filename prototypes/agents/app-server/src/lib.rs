//! A client of the Codex app-server, `codex app-server`: JSON-RPC over the app-server's stdio, or
//! over a WebSocket on the Unix socket of an app-server that runs on its own. The client starts a
//! thread, or resumes one a person works in, and hands it the waiting messages of a [`Source`]:
//! with `turn/start` while the thread is idle, and with `turn/steer` into its running turn. The
//! agent's replies come back to the client.
//!
//! It marks the messages handed over once the app-server accepts the request and puts them back if
//! it refuses it, as `docs/protocol/delivery.md`, "Device to agent", says.

use std::fs::File;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio as Pipe};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use agent_inbox::render;
use agent_source::Source;
use serde_json::{Value, json};
use tungstenite::WebSocket;

/// How long the client waits for the app-server before it looks at the source again.
pub const TICK: Duration = Duration::from_millis(100);

/// The approval policy of the threads the client starts: Codex never asks a person, and a command
/// the sandbox refuses fails back to the agent.
pub const APPROVAL_POLICY: &str = "never";

/// The sandbox of the threads the client starts: commands read files and write none, without
/// network.
pub const SANDBOX: &str = "read-only";

/// What the app-server sent within a wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Received {
    Message(String),
    Nothing,
    /// The app-server closed the connection.
    Closed,
}

/// A connection to the app-server that carries JSON-RPC messages as text.
pub trait Transport {
    fn send(&mut self, text: &str) -> io::Result<()>;

    /// The next message, waiting up to `wait` for it.
    fn receive(&mut self, wait: Duration) -> io::Result<Received>;
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn send(&mut self, text: &str) -> io::Result<()> {
        (**self).send(text)
    }

    fn receive(&mut self, wait: Duration) -> io::Result<Received> {
        (**self).receive(wait)
    }
}

/// An app-server the client runs as its child: one JSON message per line on its stdin and stdout.
pub struct Stdio {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Stdio {
    /// Starts `command`, an app-server on stdio; its stderr is the client's.
    pub fn spawn(mut command: Command) -> io::Result<Self> {
        let mut child = command.stdin(Pipe::piped()).stdout(Pipe::piped()).spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
        })
    }
}

impl Transport for Stdio {
    fn send(&mut self, text: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .expect("stdin stays open until the drop");
        writeln!(stdin, "{text}")?;
        stdin.flush()
    }

    fn receive(&mut self, wait: Duration) -> io::Result<Received> {
        Ok(match self.lines.recv_timeout(wait) {
            Ok(line) => Received::Message(line),
            Err(RecvTimeoutError::Timeout) => Received::Nothing,
            Err(RecvTimeoutError::Disconnected) => Received::Closed,
        })
    }
}

impl Drop for Stdio {
    /// Closes the app-server's stdin, which ends it, and waits for it to exit.
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

/// An app-server that listens on a Unix socket, `codex app-server --listen unix://<path>`: one JSON
/// message per WebSocket text frame.
pub struct Socket {
    socket: WebSocket<UnixStream>,
}

impl Socket {
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        let (socket, _) = tungstenite::client::client("ws://localhost/", stream)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(Self { socket })
    }
}

impl Transport for Socket {
    fn send(&mut self, text: &str) -> io::Result<()> {
        self.socket
            .send(tungstenite::Message::text(text))
            .map_err(io::Error::other)
    }

    fn receive(&mut self, wait: Duration) -> io::Result<Received> {
        self.socket.get_ref().set_read_timeout(Some(wait))?;
        frame(self.socket.read())
    }
}

/// What a read of the WebSocket with a timeout gave.
fn frame(read: tungstenite::Result<tungstenite::Message>) -> io::Result<Received> {
    match read {
        Ok(tungstenite::Message::Text(text)) => Ok(Received::Message(text.as_str().to_owned())),
        Ok(tungstenite::Message::Close(_)) => Ok(Received::Closed),
        // Pings, which tungstenite answers itself.
        Ok(_) => Ok(Received::Nothing),
        Err(tungstenite::Error::Io(error))
            if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
        {
            Ok(Received::Nothing)
        }
        Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
            Ok(Received::Closed)
        }
        Err(error) => Err(io::Error::other(error)),
    }
}

/// A transport that writes each message it carries to `log`, one JSON object a line:
/// `{"client": <message>}` for one the client sent, `{"server": <message>}` for one it received.
pub struct Logged<T> {
    transport: T,
    log: File,
}

impl<T> Logged<T> {
    pub fn new(transport: T, log: File) -> Self {
        Self { transport, log }
    }

    fn record(&mut self, side: &str, text: &str) -> io::Result<()> {
        let message: Value = serde_json::from_str(text).unwrap_or_else(|_| json!(text));
        writeln!(self.log, "{}", json!({ side: message }))
    }
}

impl<T: Transport> Transport for Logged<T> {
    fn send(&mut self, text: &str) -> io::Result<()> {
        self.record("client", text)?;
        self.transport.send(text)
    }

    fn receive(&mut self, wait: Duration) -> io::Result<Received> {
        let received = self.transport.receive(wait)?;
        if let Received::Message(text) = &received {
            self.record("server", text)?;
        }
        Ok(received)
    }
}

/// `transport`, writing each message it carries to the file `log` if one is given ([`Logged`]).
pub fn logged(transport: Box<dyn Transport>, log: Option<&str>) -> io::Result<Box<dyn Transport>> {
    Ok(match log {
        Some(log) => Box::new(Logged::new(transport, File::create(log)?)),
        None => transport,
    })
}

/// The client of one thread. It writes what happens to `events`, one JSON object a line:
/// - `{"event": "thread", "threadId"}` once it started or resumed the thread;
/// - `{"event": "turnStarted", "turnId"}` and `{"event": "turnCompleted", "turnId", "status"}`;
/// - `{"event": "commandStarted", "turnId"}` when the agent starts a shell command;
/// - `{"event": "reply", "turnId", "text"}` with each message of the agent;
/// - `{"event": "tokens", "input", "cached", "output"}` with the thread's tokens so far;
/// - `{"event": "handedOver", "via", "turnId", "msgIds"}` once the app-server accepted the waiting
///   messages in a `turn/start` or `turn/steer`;
/// - `{"event": "refused", "via", "error"}` when it refused them; they wait for the next try;
/// - `{"event": "declined", "method"}` when it asked the client something, such as an approval: the
///   client answers no request.
pub struct Client<T, W> {
    transport: T,
    events: W,
    /// Set once the client is to stop: it ends its wait for the app-server and puts back the
    /// messages it was handing over.
    stop: Arc<AtomicBool>,
    next_id: u64,
    thread: Option<String>,
    /// The thread's running turn, if it has one.
    turn: Option<String>,
}

impl<T: Transport, W: Write> Client<T, W> {
    pub fn new(transport: T, events: W, stop: Arc<AtomicBool>) -> Self {
        Self {
            transport,
            events,
            stop,
            next_id: 0,
            thread: None,
            turn: None,
        }
    }

    /// Opens the connection: `initialize`, then `initialized`.
    pub fn initialize(&mut self) -> io::Result<()> {
        self.initialize_with(json!({}))
    }

    /// As [`Client::initialize`], asking for the app-server's experimental methods too, such as
    /// `thread/queue/list`.
    pub fn initialize_experimental(&mut self) -> io::Result<()> {
        self.initialize_with(json!({ "capabilities": { "experimentalApi": true } }))
    }

    fn initialize_with(&mut self, mut params: Value) -> io::Result<()> {
        params["clientInfo"] = json!({
            "name": "skald_prototype",
            "title": "Skald prototype",
            "version": env!("CARGO_PKG_VERSION"),
        });
        self.call("initialize", params)?;
        self.send(&json!({ "method": "initialized" }))
    }

    /// Starts a thread in `cwd` that runs without a person: [`APPROVAL_POLICY`] and [`SANDBOX`];
    /// ephemeral, so Codex saves nothing of it. Returns its ID.
    pub fn start_thread(&mut self, cwd: &Path) -> io::Result<String> {
        let params = json!({
            "cwd": cwd,
            "approvalPolicy": APPROVAL_POLICY,
            "sandbox": SANDBOX,
            "ephemeral": true,
        });
        let result = self.call("thread/start", params)?;
        self.joined(&result)
    }

    /// Resumes the loaded or stored thread `thread`, such as one a person works in, with its own
    /// settings, and subscribes to its events. Returns its ID.
    pub fn resume_thread(&mut self, thread: &str) -> io::Result<String> {
        let params = json!({ "threadId": thread, "excludeTurns": true });
        let result = self.call("thread/resume", params)?;
        self.joined(&result)
    }

    fn joined(&mut self, result: &Value) -> io::Result<String> {
        let id = result["thread"]["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("the app-server gave no thread ID"))?
            .to_owned();
        self.thread = Some(id.clone());
        self.event(json!({ "event": "thread", "threadId": id }))?;
        Ok(id)
    }

    /// Starts a turn with the prompt `text`, as a person would.
    pub fn start_turn(&mut self, text: &str) -> io::Result<()> {
        let params = json!({ "threadId": self.thread, "input": input(text) });
        let result = self.call("turn/start", params)?;
        self.started(&result);
        Ok(())
    }

    /// Hands the waiting messages of `source` to the thread in one request, the text [`render`]
    /// gives them: `turn/steer` into the running turn, or `turn/start`. Marks them handed over once
    /// the app-server accepts it; puts them back if it refuses it or the client stops first.
    pub fn deliver(&mut self, source: &Source) -> io::Result<()> {
        let mut via = "";
        let handed = source.hand_over(|messages| {
            let text = render(messages);
            let (method, params) = match &self.turn {
                Some(turn) => (
                    "turn/steer",
                    json!({ "threadId": self.thread, "expectedTurnId": turn, "input": input(&text) }),
                ),
                None => (
                    "turn/start",
                    json!({ "threadId": self.thread, "input": input(&text) }),
                ),
            };
            via = method;
            match self.request(method, params)? {
                Ok(result) => {
                    if method == "turn/start" {
                        self.started(&result);
                    }
                    Ok(())
                }
                Err(error) => Err(io::Error::new(ErrorKind::ConnectionRefused, error)),
            }
        });
        match handed {
            Ok(messages) if messages.is_empty() => Ok(()),
            Ok(messages) => {
                let ids: Vec<&str> = messages.iter().map(|m| m.msg_id.as_str()).collect();
                let event = json!({ "event": "handedOver", "via": via, "turnId": self.turn, "msgIds": ids });
                self.event(event)
            }
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                self.event(json!({ "event": "refused", "via": via, "error": error.to_string() }))
            }
            Err(error) => Err(error),
        }
    }

    /// Hands the waiting messages of `source` to the thread as they come and reads its events, until
    /// the client is to stop or the app-server closes the connection.
    pub fn run(&mut self, source: &Source) -> io::Result<()> {
        let stopped = |result: io::Result<()>| match result {
            Err(error) if error.kind() == ErrorKind::Interrupted => Ok(true),
            other => other.map(|()| false),
        };
        loop {
            let received = self
                .next()
                .and_then(|message| message.map_or(Ok(()), |message| self.handle(&message)));
            if stopped(received)? || stopped(self.deliver(source))? {
                return Ok(());
            }
        }
    }

    /// The running turn learns its ID from a `turn/start` result.
    fn started(&mut self, result: &Value) {
        if result["turn"]["status"] == "inProgress" {
            self.turn = result["turn"]["id"].as_str().map(String::from);
        }
    }

    /// Sends a request and returns its result, or fails with the app-server's error.
    pub fn call(&mut self, method: &str, params: Value) -> io::Result<Value> {
        self.request(method, params)?
            .map_err(|error| io::Error::other(format!("{method} refused: {error}")))
    }

    /// Sends a request and waits for its answer: the result, or the app-server's error message.
    /// Handles the other messages that come meanwhile.
    pub fn request(&mut self, method: &str, params: Value) -> io::Result<Result<Value, String>> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "method": method, "id": id, "params": params }))?;
        loop {
            let Some(message) = self.next()? else {
                continue;
            };
            if message.get("method").is_none() && message["id"] == id {
                return Ok(match message.get("error") {
                    Some(error) => Err(error["message"]
                        .as_str()
                        .map_or_else(|| error.to_string(), String::from)),
                    None => Ok(message["result"].clone()),
                });
            }
            self.handle(&message)?;
        }
    }

    /// The next message, or `None` after a tick without one or a message that is not JSON. Fails
    /// with [`ErrorKind::Interrupted`] once the client is to stop, and when the app-server closed
    /// the connection.
    fn next(&mut self) -> io::Result<Option<Value>> {
        if self.stop.load(Ordering::SeqCst) {
            return Err(io::Error::new(ErrorKind::Interrupted, "stopped"));
        }
        match self.transport.receive(TICK)? {
            Received::Message(text) => Ok(serde_json::from_str(&text).ok()),
            Received::Nothing => Ok(None),
            Received::Closed => Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "the app-server closed the connection",
            )),
        }
    }

    /// Acts on a request or notification of the app-server: notifications of other threads are
    /// ignored.
    fn handle(&mut self, message: &Value) -> io::Result<()> {
        let Some(method) = message["method"].as_str() else {
            return Ok(());
        };
        if let Some(id) = message.get("id") {
            let refusal = json!({ "code": -32601, "message": "the client answers no requests" });
            self.send(&json!({ "id": id, "error": refusal }))?;
            return self.event(json!({ "event": "declined", "method": method }));
        }
        let params = &message["params"];
        if self.thread.is_none() || params["threadId"].as_str() != self.thread.as_deref() {
            return Ok(());
        }
        let item = &params["item"];
        match (method, item["type"].as_str()) {
            ("turn/started", _) => {
                let turn = &params["turn"]["id"];
                self.turn = turn.as_str().map(String::from);
                self.event(json!({ "event": "turnStarted", "turnId": turn }))
            }
            ("turn/completed", _) => {
                let turn = &params["turn"];
                if self.turn.as_deref() == turn["id"].as_str() {
                    self.turn = None;
                }
                let event = json!({ "event": "turnCompleted", "turnId": turn["id"], "status": turn["status"] });
                self.event(event)
            }
            ("item/started", Some("commandExecution")) => {
                self.event(json!({ "event": "commandStarted", "turnId": params["turnId"] }))
            }
            ("item/completed", Some("agentMessage")) => {
                let event =
                    json!({ "event": "reply", "turnId": params["turnId"], "text": item["text"] });
                self.event(event)
            }
            ("thread/tokenUsage/updated", _) => {
                let total = &params["tokenUsage"]["total"];
                let event = json!({
                    "event": "tokens",
                    "input": total["inputTokens"],
                    "cached": total["cachedInputTokens"],
                    "output": total["outputTokens"],
                });
                self.event(event)
            }
            _ => Ok(()),
        }
    }

    fn send(&mut self, message: &Value) -> io::Result<()> {
        self.transport.send(&message.to_string())
    }

    fn event(&mut self, event: Value) -> io::Result<()> {
        writeln!(self.events, "{event}")?;
        self.events.flush()
    }
}

/// The input of a turn: one text.
fn input(text: &str) -> Value {
    json!([{ "type": "text", "text": text }])
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// A transport that gives the messages in `incoming` and records what the client sends.
    struct Script {
        incoming: VecDeque<String>,
        sent: Vec<Value>,
    }

    impl Transport for Script {
        fn send(&mut self, text: &str) -> io::Result<()> {
            self.sent.push(serde_json::from_str(text).unwrap());
            Ok(())
        }

        fn receive(&mut self, _: Duration) -> io::Result<Received> {
            Ok(self
                .incoming
                .pop_front()
                .map_or(Received::Closed, Received::Message))
        }
    }

    /// A client of the thread `t1` that receives `incoming`, with its events.
    fn client(incoming: &[&str]) -> Client<Script, Vec<u8>> {
        let script = Script {
            incoming: incoming.iter().map(|text| (*text).to_owned()).collect(),
            sent: Vec::new(),
        };
        let mut client = Client::new(script, Vec::new(), Arc::default());
        client.thread = Some("t1".to_owned());
        client
    }

    /// Runs `test` and fails if it takes over 5 s: a client that misses the end of the
    /// connection would wait for ever.
    fn bounded(test: impl FnOnce() + Send + 'static) {
        let (done, finished) = mpsc::channel();
        thread::spawn(move || {
            test();
            let _ = done.send(());
        });
        finished
            .recv_timeout(Duration::from_secs(5))
            .expect("the test ends within 5 s");
    }

    fn events(client: &Client<Script, Vec<u8>>) -> Vec<Value> {
        String::from_utf8_lossy(&client.events)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    // Shortened from Codex 0.162.1 with the approval policy `untrusted`.
    const APPROVAL: &str = r#"{"method":"item/commandExecution/requestApproval","id":0,"params":{"kind":"command","threadId":"t1","turnId":"u1","itemId":"call_0","command":"/bin/zsh -lc 'sleep 5'","availableDecisions":["accept","cancel"]}}"#;

    #[test]
    fn declines_what_the_app_server_asks() {
        let mut client = client(&[]);
        client
            .handle(&serde_json::from_str(APPROVAL).unwrap())
            .unwrap();
        assert_eq!(
            events(&client),
            [json!({ "event": "declined", "method": "item/commandExecution/requestApproval" })]
        );
        assert_eq!(
            client.transport.sent,
            [
                json!({ "id": 0, "error": { "code": -32601, "message": "the client answers no requests" } })
            ]
        );
    }

    #[test]
    fn follows_the_turns_of_its_own_thread_only() {
        let notification = |method: &str, thread: &str, params: Value| {
            let mut params = params;
            params["threadId"] = json!(thread);
            json!({ "method": method, "params": params })
        };
        let started = |thread: &str, turn: &str| {
            notification("turn/started", thread, json!({ "turn": { "id": turn } }))
        };
        let completed = |thread: &str, turn: &str| {
            notification(
                "turn/completed",
                thread,
                json!({ "turn": { "id": turn, "status": "completed" } }),
            )
        };
        let mut client = client(&[]);
        for message in [
            started("t2", "u2"),
            notification("thread/tokenUsage/updated", "t2", json!({})),
            started("t1", "u1"),
            completed("t1", "u0"),
        ] {
            client.handle(&message).unwrap();
        }
        assert_eq!(
            client.turn.as_deref(),
            Some("u1"),
            "u0 is not the running turn"
        );
        for message in [
            notification(
                "item/started",
                "t1",
                json!({ "turnId": "u1", "item": { "type": "commandExecution", "command": "sleep 5" } }),
            ),
            notification(
                "item/started",
                "t1",
                json!({ "turnId": "u1", "item": { "type": "agentMessage", "text": "" } }),
            ),
            notification(
                "item/completed",
                "t1",
                json!({ "turnId": "u1", "item": { "type": "agentMessage", "text": "OK" } }),
            ),
            notification(
                "thread/tokenUsage/updated",
                "t1",
                json!({ "turnId": "u1", "tokenUsage": {
                    "total": { "inputTokens": 13_316, "cachedInputTokens": 12_672, "outputTokens": 20 },
                    "last": { "inputTokens": 1, "cachedInputTokens": 0, "outputTokens": 1 },
                } }),
            ),
            completed("t2", "u1"),
        ] {
            client.handle(&message).unwrap();
        }
        assert_eq!(client.turn.as_deref(), Some("u1"), "t2's turn ended");
        client.handle(&completed("t1", "u1")).unwrap();
        assert_eq!(client.turn, None);
        assert_eq!(
            events(&client),
            [
                json!({ "event": "turnStarted", "turnId": "u1" }),
                json!({ "event": "turnCompleted", "turnId": "u0", "status": "completed" }),
                json!({ "event": "commandStarted", "turnId": "u1" }),
                json!({ "event": "reply", "turnId": "u1", "text": "OK" }),
                json!({ "event": "tokens", "input": 13_316, "cached": 12_672, "output": 20 }),
                json!({ "event": "turnCompleted", "turnId": "u1", "status": "completed" }),
            ]
        );
    }

    #[test]
    fn a_request_takes_only_its_own_answer() {
        bounded(|| {
            // The app-server numbers its requests apart from the client's.
            let mut client = client(&[
                APPROVAL,
                r#"{"id":5,"result":{"other":true}}"#,
                r#"{"id":0,"result":{"turn":{"id":"u1"}}}"#,
                r#"{"id":1,"error":{"code":-32600,"message":"no active turn to steer"}}"#,
            ]);
            let result = client.request("turn/start", json!({})).unwrap();
            assert_eq!(result, Ok(json!({ "turn": { "id": "u1" } })));
            assert_eq!(events(&client).len(), 1, "the approval was declined");
            assert_eq!(
                client.request("turn/steer", json!({})).unwrap(),
                Err("no active turn to steer".to_owned())
            );
            let ids: Vec<&Value> = client.transport.sent.iter().map(|m| &m["id"]).collect();
            assert_eq!(
                ids,
                [&json!(0), &json!(0), &json!(1)],
                "the request, the approval's answer, the next request"
            );
        });
    }

    #[test]
    fn reads_the_frames_of_the_websocket() {
        let io = |kind: ErrorKind| Err(tungstenite::Error::Io(io::Error::from(kind)));
        let text = Ok(tungstenite::Message::text("{}"));
        assert_eq!(frame(text).unwrap(), Received::Message("{}".to_owned()));
        assert_eq!(
            frame(Ok(tungstenite::Message::Close(None))).unwrap(),
            Received::Closed
        );
        let ping = Ok(tungstenite::Message::Ping(Vec::new().into()));
        assert_eq!(frame(ping).unwrap(), Received::Nothing);
        assert_eq!(frame(io(ErrorKind::WouldBlock)).unwrap(), Received::Nothing);
        assert_eq!(frame(io(ErrorKind::TimedOut)).unwrap(), Received::Nothing);
        assert_eq!(
            frame(Err(tungstenite::Error::ConnectionClosed)).unwrap(),
            Received::Closed
        );
        assert_eq!(
            frame(Err(tungstenite::Error::AlreadyClosed)).unwrap(),
            Received::Closed
        );
        assert!(frame(io(ErrorKind::ConnectionReset)).is_err());
    }

    #[test]
    fn learns_the_running_turn_from_a_turn_start_result() {
        let mut client = client(&[]);
        client.started(&json!({ "turn": { "id": "u1", "status": "completed" } }));
        assert_eq!(client.turn, None);
        client.started(&json!({ "turn": { "id": "u1", "status": "inProgress" } }));
        assert_eq!(client.turn.as_deref(), Some("u1"));
    }

    #[test]
    fn a_closed_connection_or_a_stop_ends_a_request() {
        bounded(|| {
            let mut client = client(&[]);
            let error = client.request("turn/start", json!({})).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
            client.stop.store(true, Ordering::SeqCst);
            let error = client.request("turn/start", json!({})).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Interrupted);
        });
    }
}
