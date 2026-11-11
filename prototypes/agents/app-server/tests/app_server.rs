//! `agent-app-server` against stand-ins that replay exchanges recorded from the Codex 0.162.1
//! app-server on the model stub, shortened, in `tests/exchanges/`: a stand-in `codex app-server`
//! on stdio, and an app-server on a Unix socket. `refused.jsonl` joins lines of `busy.jsonl` and
//! `idle.jsonl` with the error Codex gives a `turn/steer` after its turn ended.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use agent_source::{Message, Source};
use agent_testing::{
    Exchange, assert_carries_once, assert_failed, assert_handed_over, assert_waiting, stderr,
};
use serde_json::{Value, json};

/// The exchange recorded in `tests/exchanges/<name>`.
fn exchange(name: &str) -> Exchange {
    Exchange::read(&Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/exchanges/{name}")))
}

/// What the client sent the app-server, and, when each message came, the messages handed over.
struct Received {
    messages: Vec<Value>,
    handed: Vec<String>,
}

impl Received {
    fn methods(&self) -> Vec<&str> {
        self.messages
            .iter()
            .map(|message| message["method"].as_str().unwrap_or("<answer>"))
            .collect()
    }

    /// The `n`-th message with `method`, and the messages handed over when it came.
    fn nth(&self, method: &str, n: usize) -> (&Value, &str) {
        let index = self
            .messages
            .iter()
            .enumerate()
            .filter(|(_, message)| message["method"] == method)
            .nth(n)
            .unwrap_or_else(|| panic!("no {method} #{n}: {:?}", self.methods()))
            .0;
        (&self.messages[index], &self.handed[index])
    }
}

/// Asserts that the client sent the exchange's requests and notifications, in its order, with its
/// IDs.
fn assert_sent_as_recorded(received: &Received, exchange: &Exchange) {
    let recorded: Vec<(&Value, &Value)> = exchange
        .client()
        .iter()
        .map(|message| (&message["method"], &message["id"]))
        .collect();
    let sent: Vec<(&Value, &Value)> = received
        .messages
        .iter()
        .take(recorded.len())
        .map(|message| (&message["method"], &message["id"]))
        .collect();
    assert_eq!(sent, recorded);
}

/// The text of a `turn/start` or `turn/steer` request's one input.
fn text(request: &Value) -> &str {
    let input = request["params"]["input"].as_array().unwrap();
    assert_eq!(input.len(), 1, "{request}");
    assert_eq!(input[0]["type"], "text");
    input[0]["text"].as_str().unwrap()
}

/// A stand-in `codex` in `bin` that replays `exchange` as `codex app-server` on stdio: it records
/// its arguments, and each line it reads in `received/<n>` with the names in the source's `handed`
/// directory then, and answers it with the server's messages that follow it in the exchange. A
/// second after its stdin ends, it writes `ended` and exits.
struct StandIn {
    dir: tempfile::TempDir,
}

impl StandIn {
    fn new(exchange: &Exchange, source: &Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        exchange.stand_in(
            dir.path(),
            &format!(
                "ls '{}' >\"$dir/received/$n.handed\"",
                source.join("handed").display()
            ),
            "exec 2>&-\nsleep 1\ntouch \"$dir/ended\"",
        );
        Self { dir }
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    /// Whether the stand-in ended after its stdin did.
    fn ended(&self) -> bool {
        self.dir.path().join("ended").exists()
    }

    fn args(&self) -> Vec<String> {
        let args = fs::read_to_string(self.dir.path().join("args")).unwrap();
        args.lines().map(String::from).collect()
    }

    fn received(&self) -> Received {
        let dir = self.dir.path().join("received");
        // Each line's `.handed` file comes after the line's own.
        let count = fs::read_dir(&dir)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .to_string_lossy()
                    .ends_with(".handed")
            })
            .count();
        let read = |name: String| fs::read_to_string(dir.join(name)).unwrap();
        Received {
            messages: (0..count)
                .map(|n| serde_json::from_str(&read(n.to_string())).unwrap())
                .collect(),
            handed: (0..count).map(|n| read(format!("{n}.handed"))).collect(),
        }
    }
}

/// A running `agent-app-server` and the events it printed.
struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    events: Receiver<Value>,
    seen: Vec<Value>,
}

impl Running {
    /// Starts `agent-app-server <source> <args>` in `cwd` with only `PATH` in its environment:
    /// `bin`, if given, then the system's for the stand-in's shell.
    fn start(source: &Path, args: &[&str], cwd: &Path, bin: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-app-server"));
        command.arg(source).args(args).current_dir(cwd).env_clear();
        if let Some(bin) = bin {
            command.env("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, events) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let event = serde_json::from_str(&line.unwrap()).expect("events are JSON");
                if sender.send(event).is_err() {
                    break;
                }
            }
        });
        Self {
            stdin: child.stdin.take(),
            child,
            events,
            seen: Vec::new(),
        }
    }

    /// Waits up to 10 s for an event that `wanted` holds for.
    fn wait_for(&mut self, wanted: impl Fn(&Value) -> bool) {
        while !self.seen.iter().any(&wanted) {
            let event = self
                .events
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("no such event: {:?}", self.seen));
            self.seen.push(event);
        }
    }

    /// Ends the client's stdin, which stops it, and returns its output, with the events it printed.
    fn stop(mut self) -> (Output, Vec<Value>) {
        drop(self.stdin.take());
        self.exit()
    }

    /// Waits for the events named `names`, in order, then stops the client, asserts that it
    /// succeeded and returns every event it printed.
    fn stop_after(mut self, names: &[&'static str]) -> Vec<Value> {
        for name in names {
            self.wait_for(event(name));
        }
        let (output, events) = self.stop();
        assert!(output.status.success(), "{}", stderr(&output));
        events
    }

    /// Waits for the client to exit on its own, its stdin still open.
    fn stop_after_exit(self) -> (Output, Vec<Value>) {
        self.exit()
    }

    /// Waits up to 10 s for the client to exit, and returns its output and every event it printed.
    fn exit(mut self) -> (Output, Vec<Value>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                self.child.kill().unwrap();
                self.child.wait().unwrap();
                panic!("agent-app-server did not exit: {:?}", self.seen);
            }
            thread::sleep(Duration::from_millis(20));
        };
        drop(self.stdin.take());
        let mut stderr = Vec::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        self.seen.extend(self.events.iter());
        let output = Output {
            status,
            stdout: Vec::new(),
            stderr,
        };
        (output, self.seen)
    }
}

fn event(name: &'static str) -> impl Fn(&Value) -> bool {
    move |event| event["event"] == name
}

/// The events named `name`.
fn named<'a>(events: &'a [Value], name: &str) -> Vec<&'a Value> {
    events
        .iter()
        .filter(|event| event["event"] == name)
        .collect()
}

fn msg_ids(messages: &[Message]) -> Value {
    json!(messages.iter().map(|m| &m.msg_id).collect::<Vec<_>>())
}

/// A source with two waiting messages, in a directory that also holds an empty project.
fn waiting() -> (tempfile::TempDir, Source, [Message; 2]) {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("project")).unwrap();
    let source = Source::open(dir.path().join("source")).unwrap();
    let messages = [
        source.add("alice of skald", "first").unwrap(),
        source.add("bob", "second").unwrap(),
    ];
    (dir, source, messages)
}

/// A replay of an exchange by a stand-in `codex`, with two messages waiting in its source.
struct Replay {
    dir: tempfile::TempDir,
    source: Source,
    messages: [Message; 2],
    codex: StandIn,
}

impl Replay {
    fn new(exchange: &Exchange) -> Self {
        let (dir, source, messages) = waiting();
        let codex = StandIn::new(exchange, &dir.path().join("source"));
        Self {
            dir,
            source,
            messages,
            codex,
        }
    }

    /// Starts `agent-app-server <source> <args>` in the project, on the stand-in.
    fn start(&self, args: &[&str]) -> Running {
        Running::start(
            &self.dir.path().join("source"),
            args,
            &self.dir.path().join("project"),
            Some(&self.codex.bin()),
        )
    }
}

#[test]
fn starts_a_thread_and_hands_the_waiting_messages_to_it_with_turn_start() {
    let exchange = exchange("idle.jsonl");
    let replay = Replay::new(&exchange);
    let log = replay.dir.path().join("exchange.jsonl");
    let args = ["--log", log.to_str().unwrap(), "--", "-c", "a=1"];
    let events = replay.start(&args).stop_after(&["turnCompleted"]);
    let Replay {
        dir,
        source,
        messages,
        codex,
    } = &replay;
    let messages: &[Message] = messages;
    assert!(codex.ended(), "the client exits once its app-server did");
    assert_eq!(codex.args(), ["app-server", "-c", "a=1"]);

    let received = codex.received();
    assert_sent_as_recorded(&received, &exchange);
    assert_eq!(received.messages.len(), exchange.client().len());
    let (start, _) = received.nth("thread/start", 0);
    let project = fs::canonicalize(dir.path().join("project")).unwrap();
    assert_eq!(
        start["params"],
        json!({
            "cwd": project,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "ephemeral": true,
        })
    );
    let (turn, handed) = received.nth("turn/start", 0);
    let thread = exchange.thread();
    assert_eq!(turn["params"]["threadId"], thread);
    assert_carries_once(text(turn), messages);
    assert_eq!(
        handed, "",
        "nothing was handed over before the app-server accepted it"
    );
    assert_handed_over(source, messages);

    let recorded_turn = exchange
        .0
        .iter()
        .find_map(|line| line["server"]["result"]["turn"]["id"].as_str());
    assert_eq!(
        named(&events, "handedOver"),
        [&json!({
            "event": "handedOver",
            "via": "turn/start",
            "turnId": recorded_turn,
            "msgIds": msg_ids(messages),
        })]
    );
    assert_eq!(
        named(&events, "thread"),
        [&json!({ "event": "thread", "threadId": thread })]
    );
    let replies = named(&events, "reply");
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["text"], "Done.");
    assert_eq!(replies[0]["turnId"], json!(recorded_turn));
    assert_eq!(named(&events, "turnCompleted")[0]["status"], "completed");
    assert!(named(&events, "tokens").len() == 1);

    // The log holds every message of the exchange, in its order.
    let logged: Vec<Value> = fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let sides = |lines: &[Value]| -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                let (side, message) = line.as_object().unwrap().iter().next().unwrap();
                format!("{side} {} {}", message["method"], message["id"])
            })
            .collect()
    };
    assert_eq!(sides(&logged), sides(&exchange.0));
    assert_eq!(logged[3]["client"], received.messages[2]);
}

#[test]
fn steers_the_waiting_messages_into_the_running_turn() {
    let exchange = exchange("busy.jsonl");
    let replay = Replay::new(&exchange);
    let prompt = "Run the shell command `sleep 20`. Then reply with only OK.";
    let events = replay
        .start(&["--prompt", prompt])
        .stop_after(&["turnCompleted"]);
    let Replay {
        source,
        messages,
        codex,
        ..
    } = &replay;
    let messages: &[Message] = messages;
    assert_eq!(codex.args(), ["app-server"]);

    let received = codex.received();
    assert_sent_as_recorded(&received, &exchange);
    let (first, _) = received.nth("turn/start", 0);
    assert_eq!(text(first), prompt);
    let (steer, handed) = received.nth("turn/steer", 0);
    let turn = exchange
        .0
        .iter()
        .find_map(|line| line["server"]["result"]["turn"]["id"].as_str());
    assert_eq!(steer["params"]["threadId"], exchange.thread());
    assert_eq!(steer["params"]["expectedTurnId"], json!(turn));
    assert_carries_once(text(steer), messages);
    assert_eq!(
        handed, "",
        "nothing was handed over before the app-server accepted it"
    );
    assert_handed_over(source, messages);

    assert_eq!(
        named(&events, "handedOver"),
        [
            &json!({ "event": "handedOver", "via": "turn/steer", "turnId": turn, "msgIds": msg_ids(messages) })
        ]
    );
    assert_eq!(
        named(&events, "turnStarted"),
        [&json!({ "event": "turnStarted", "turnId": turn })]
    );
    assert_eq!(
        named(&events, "commandStarted"),
        [&json!({ "event": "commandStarted", "turnId": turn })]
    );
    assert_eq!(named(&events, "reply")[0]["turnId"], json!(turn));
    assert_eq!(
        named(&events, "turnCompleted"),
        [&json!({ "event": "turnCompleted", "turnId": turn, "status": "completed" })]
    );
}

#[test]
fn a_refused_steer_leaves_the_messages_waiting_for_the_next_turn() {
    let exchange = exchange("refused.jsonl");
    let replay = Replay::new(&exchange);
    let events = replay
        .start(&["--prompt", "Reply with only OK."])
        .stop_after(&["handedOver", "turnCompleted"]);
    let Replay {
        source,
        messages,
        codex,
        ..
    } = &replay;
    let messages: &[Message] = messages;

    let received = codex.received();
    assert_sent_as_recorded(&received, &exchange);
    let (steer, _) = received.nth("turn/steer", 0);
    assert_carries_once(text(steer), messages);
    // The prompt's turn/start, then the messages' after the refused steer.
    let (start, handed) = received.nth("turn/start", 1);
    assert_carries_once(text(start), messages);
    assert_eq!(handed, "", "the refused steer marked nothing");
    assert_handed_over(source, messages);
    assert_eq!(
        named(&events, "refused"),
        [&json!({ "event": "refused", "via": "turn/steer", "error": "no active turn to steer" })]
    );
    assert_eq!(named(&events, "handedOver")[0]["via"], "turn/start");
}

#[test]
fn messages_refused_until_the_client_stops_stay_waiting() {
    let exchange = exchange("refused.jsonl").until(|line| line["server"].get("error").is_some());
    let replay = Replay::new(&exchange);
    let mut client = replay.start(&["--prompt", "Reply with only OK."]);
    client.wait_for(event("refused"));
    // The next steer, which the stand-in never answers.
    let deadline = Instant::now() + Duration::from_secs(10);
    while replay
        .codex
        .received()
        .methods()
        .iter()
        .filter(|m| **m == "turn/steer")
        .count()
        < 2
    {
        assert!(Instant::now() < deadline, "no second steer");
        thread::sleep(Duration::from_millis(20));
    }
    let events = client.stop_after(&[]);
    assert_waiting(&replay.source, &replay.messages);
    assert!(named(&events, "handedOver").is_empty(), "{events:?}");
    // The second steer waited for its answer when the client stopped: no refusal.
    assert_eq!(named(&events, "refused").len(), 1, "{events:?}");
}

#[test]
fn fails_when_the_app_server_refuses_the_thread_or_closes() {
    let idle = || exchange("idle.jsonl");
    // thread/start refused.
    let refused = Exchange(
        idle()
            .until(|line| line["client"]["method"] == "thread/start")
            .0
            .into_iter()
            .chain([json!({ "server": { "id": 1, "error": { "code": -32600, "message": "no" } } })])
            .collect(),
    );
    let closed = "agent-app-server: the app-server closed the connection\n";
    // The stand-in closes its stdout once it answered that many lines: initialize, or the whole
    // exchange, so the client sees it close while it waits for messages.
    for (exchange, closes_after, error, handed) in [
        (
            refused,
            None,
            "agent-app-server: thread/start refused: no\n",
            false,
        ),
        (
            idle().until(|line| line["server"]["id"] == 0),
            Some(1),
            closed,
            false,
        ),
        (idle(), Some(4), closed, true),
    ] {
        let replay = Replay::new(&exchange);
        if let Some(lines) = closes_after {
            let script = replay.codex.bin().join("codex");
            let text = fs::read_to_string(&script).unwrap();
            let closing = format!("n=$((n + 1)); if [ $n = {lines} ]; then exec 1>&-; fi");
            fs::write(&script, text.replace("n=$((n + 1))", &closing)).unwrap();
        }
        let (output, _) = replay.start(&[]).stop_after_exit();
        assert_eq!(stderr(&output), error, "{closes_after:?}");
        assert_eq!(output.status.code(), Some(1));
        if handed {
            assert_handed_over(&replay.source, &replay.messages);
        } else {
            assert_waiting(&replay.source, &replay.messages);
        }
    }

    // No app-server to run.
    let (dir, _, _) = waiting();
    let client = Running::start(
        &dir.path().join("source"),
        &[],
        &dir.path().join("project"),
        None,
    );
    assert_failed(&client.stop_after_exit().0, "agent-app-server: ");
}

/// An app-server on a Unix socket that replays `exchange`: one WebSocket text frame a message. It
/// returns the messages it received, once the client closed the connection.
fn socket_stand_in(exchange: Exchange, socket: &Path) -> thread::JoinHandle<Vec<Value>> {
    let listener = UnixListener::bind(socket).unwrap();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        let answers = exchange.answers();
        let mut received = Vec::new();
        while let Ok(message) = socket.read() {
            let tungstenite::Message::Text(text) = message else {
                continue;
            };
            received.push(serde_json::from_str(text.as_str()).unwrap());
            for answer in answers.get(received.len() - 1).into_iter().flatten() {
                socket
                    .send(tungstenite::Message::text(answer.to_string()))
                    .unwrap();
            }
        }
        received
    })
}

#[test]
fn resumes_a_thread_on_an_app_server_on_a_socket_and_hands_the_messages_to_it() {
    let exchange = exchange("person.jsonl");
    let thread = exchange.thread();
    let (dir, source, messages) = waiting();
    let socket = dir.path().join("s.sock");
    let server = socket_stand_in(Exchange(exchange.0.clone()), &socket);
    // Without PATH: the client runs no app-server.
    let mut client = Running::start(
        &dir.path().join("source"),
        &["--socket", socket.to_str().unwrap(), "--thread", &thread],
        &dir.path().join("project"),
        None,
    );
    client.wait_for(event("turnCompleted"));
    let (output, events) = client.stop();
    assert!(output.status.success(), "{}", stderr(&output));

    let received = server.join().unwrap();
    let methods: Vec<&Value> = received.iter().map(|m| &m["method"]).collect();
    let recorded: Vec<&Value> = exchange.client().iter().map(|m| &m["method"]).collect();
    assert_eq!(methods, recorded);
    assert_eq!(
        received[2]["params"],
        json!({ "threadId": thread, "excludeTurns": true }),
        "the person's thread, with its own settings"
    );
    assert_eq!(received[3]["params"]["threadId"], thread);
    assert_carries_once(text(&received[3]), &messages);
    assert_handed_over(&source, &messages);
    assert_eq!(
        named(&events, "thread"),
        [&json!({ "event": "thread", "threadId": thread })]
    );
    assert_eq!(named(&events, "handedOver")[0]["via"], "turn/start");
    assert_eq!(named(&events, "reply").len(), 1);
}

#[test]
fn wrong_arguments_print_the_usage() {
    for args in [
        &[][..],
        &["--socket", "s"],
        &["dir", "--socket"],
        &["dir", "--thread", "a", "--thread", "b"],
        &["dir", "--verbose", "x"],
        &["dir", "--socket", "s", "--", "-c", "a=1"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-app-server"))
            .args(args)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(
            stderr(&output).starts_with("usage: agent-app-server"),
            "{args:?}"
        );
    }
}
