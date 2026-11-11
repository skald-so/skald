//! The live check of the Codex app-server with the `agent-app-server` client: the app-server's
//! options, the arguments of the client and of the terminal UI a person would use, what the client
//! printed, and the checks' verdicts. The binary starts the processes.

use std::path::Path;

use serde_json::{Value, json};

use crate::channel::{Files, Sent};
use crate::{Agent, Checks, Kind, Options, StubLog, Usage, codex_config, handed_over};
use crate::{handover_delay, queue, seconds};

/// The client's binary, next to `live-check`.
pub const CLIENT_BIN: &str = "agent-app-server";

/// The prompt of the terminal UI's session, the one a person works in: it starts the session's
/// thread.
pub fn person_prompt() -> String {
    queue::prompt(Kind::Idle)
}

/// The options of an app-server, on the model stub at `stub` if given: no plugins, notification
/// program or skills' instructions, `model` if given, and the lowest effort. The app-server has no
/// option that ignores the user's config.
pub fn options(model: Option<&str>, stub: Option<&str>) -> Vec<String> {
    let mut config = vec!["notify=[]".to_owned()];
    if let Some(model) = model {
        config.push(format!("model={}", json!(model)));
    }
    config.extend(codex_config(stub));
    let mut args = vec!["--disable".to_owned(), "plugins".to_owned()];
    for value in config {
        args.extend(["-c".to_owned(), value]);
    }
    args
}

/// The arguments of the app-server that listens on `socket`, with `options` and `project` trusted,
/// so the terminal UI opens no trust dialog there.
pub fn listening(socket: &Path, project: &Path, options: &[String]) -> Vec<String> {
    let mut args = vec![
        "app-server".to_owned(),
        "--listen".to_owned(),
        format!("unix://{}", socket.display()),
    ];
    args.extend(options.iter().cloned());
    args.extend([
        "-c".to_owned(),
        format!(
            "projects={{{}={{trust_level=\"trusted\"}}}}",
            json!(project.to_string_lossy())
        ),
    ]);
    args
}

/// Where the client finds the app-server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Connect<'a> {
    /// Its own app-server with these options.
    Own(Vec<String>),
    /// The app-server on this socket: a new thread, or the thread a person works in.
    Socket {
        path: &'a Path,
        thread: Option<&'a str>,
    },
}

/// The arguments of `agent-app-server` for the session in `files`: its source, the JSON-RPC
/// messages logged to `exchange.jsonl` in its directory, the app-server it connects to and the
/// first turn's `prompt`, if any.
pub fn client(files: &Files, connect: &Connect, prompt: Option<&str>) -> Vec<String> {
    let mut args = vec![
        files.source.to_string_lossy().into_owned(),
        "--log".to_owned(),
        exchange(files).to_string_lossy().into_owned(),
    ];
    if let Some(prompt) = prompt {
        args.extend(["--prompt".to_owned(), prompt.to_owned()]);
    }
    match connect {
        Connect::Own(options) => {
            args.push("--".to_owned());
            args.extend(options.iter().cloned());
        }
        Connect::Socket { path, thread } => {
            args.extend(["--socket".to_owned(), path.to_string_lossy().into_owned()]);
            if let Some(thread) = thread {
                args.extend(["--thread".to_owned(), (*thread).to_owned()]);
            }
        }
    }
    args
}

/// The client's log of JSON-RPC messages.
pub fn exchange(files: &Files) -> std::path::PathBuf {
    files.dir.join("exchange.jsonl")
}

/// The arguments of the terminal UI a person would use, connected to the app-server on `socket`:
/// in `project`, with `model` if given, and the person's prompt.
pub fn person(socket: &Path, project: &Path, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--remote".to_owned(),
        format!("unix://{}", socket.display()),
        "-C".to_owned(),
        project.to_string_lossy().into_owned(),
    ];
    if let Some(model) = model {
        args.extend(["-m".to_owned(), model.to_owned()]);
    }
    args.push(person_prompt());
    args
}

/// The thread of the person's session in a `thread/list` result: the one whose first message is
/// the person's prompt, once it is idle.
pub fn person_thread(list: &Value) -> Option<String> {
    list["data"].as_array()?.iter().find_map(|thread| {
        (thread["preview"] == person_prompt() && thread["status"]["type"] == "idle")
            .then(|| thread["id"].as_str().map(String::from))
            .flatten()
    })
}

/// A handover the client printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    /// `turn/start` or `turn/steer`.
    pub via: String,
    pub turn: String,
    pub at_ms: u64,
}

/// A reply of the agent the client printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub turn: String,
    pub text: String,
    pub at_ms: u64,
}

/// What one run of the client printed; each session of the check has one message.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Delivery {
    /// When the client started, in milliseconds since the Unix epoch.
    pub started_ms: u64,
    pub thread: Option<String>,
    /// The turns that started, in order.
    pub turns: Vec<String>,
    pub completed: Vec<String>,
    pub handover: Option<Handover>,
    pub refused: usize,
    /// The app-server's requests the client declined.
    pub declined: Vec<String>,
    pub replies: Vec<Reply>,
    pub command_started_ms: Option<u64>,
    /// The thread's tokens, if the app-server says.
    pub usage: Option<Usage>,
    /// What the model stub saw; `None` on the real model.
    pub stub: Option<StubLog>,
}

impl Delivery {
    pub fn new(started_ms: u64) -> Self {
        Self {
            started_ms,
            ..Self::default()
        }
    }

    /// Records the client's output `line`, printed at `at_ms`.
    pub fn record(&mut self, line: &str, at_ms: u64) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let text = |key: &str| event[key].as_str().unwrap_or_default().to_owned();
        match event["event"].as_str() {
            Some("thread") => self.thread = Some(text("threadId")),
            Some("turnStarted") => self.turns.push(text("turnId")),
            Some("turnCompleted") => self.completed.push(text("turnId")),
            Some("commandStarted") => {
                self.command_started_ms.get_or_insert(at_ms);
            }
            Some("reply") => self.replies.push(Reply {
                turn: text("turnId"),
                text: text("text"),
                at_ms,
            }),
            Some("handedOver") => {
                self.handover.get_or_insert(Handover {
                    via: text("via"),
                    turn: text("turnId"),
                    at_ms,
                });
            }
            Some("refused") => self.refused += 1,
            Some("declined") => self.declined.push(text("method")),
            Some("tokens") => {
                let count = |key: &str| event[key].as_u64().unwrap_or(0);
                self.usage = Some(Usage {
                    input: count("input"),
                    cached: count("cached"),
                    output: count("output"),
                });
            }
            _ => {}
        }
    }

    /// The agent's last reply in the turn that took the message, once that turn completed.
    pub fn answer(&self) -> Option<&Reply> {
        let turn = &self.handover.as_ref()?.turn;
        if !self.completed.contains(turn) {
            return None;
        }
        self.replies.iter().rev().find(|reply| reply.turn == *turn)
    }

    /// Whether `token` reached the model: in a request to the stub, or in the reply of the turn
    /// that took the message.
    pub fn got(&self, token: &str) -> bool {
        match &self.stub {
            Some(stub) => stub.saw(token),
            None => self
                .answer()
                .is_some_and(|reply| reply.text.contains(token)),
        }
    }

    /// Whether the stub got no credential but the placeholder, and the client declined no request
    /// of the app-server, such as an approval.
    fn clean(&self) -> bool {
        self.declined.is_empty()
            && self
                .stub
                .as_ref()
                .is_none_or(|stub| stub.failure().is_none())
    }

    pub fn summary(&self) -> String {
        let replies: Vec<&str> = self.replies.iter().map(|r| r.text.trim()).collect();
        let mut summary = format!("replies {replies:?}");
        if self.refused > 0 {
            summary.push_str(&format!(", refused {} time(s)", self.refused));
        }
        if !self.declined.is_empty() {
            summary.push_str(&format!(", declined {:?}", self.declined));
        }
        match (&self.stub, self.usage) {
            (Some(stub), _) => {
                summary.push_str(&format!(", requests to the stub: {}", stub.requests.len()));
                if let Some(failure) = stub.failure() {
                    summary.push_str(&format!(", {failure}"));
                }
            }
            (None, Some(usage)) => summary.push_str(&format!(", {}", usage.summary())),
            (None, None) => {}
        }
        summary
    }

    /// When the message was handed over and the turn that took it answered, after `from_ms`.
    fn delays(&self, sent: &Sent, from_ms: u64) -> String {
        let answered = self.answer().map_or_else(
            || "never".to_owned(),
            |reply| seconds(reply.at_ms.saturating_sub(from_ms)),
        );
        let via = self.handover.as_ref().map_or("no request", |h| &h.via);
        format!(
            "handed over in {via} {} and answered {answered} after",
            handover_delay(sent.state, from_ms)
        )
    }

    /// The common part of every check: the message with `sent.token` went in with `via`, is handed
    /// over, reached the model and its turn answered.
    fn delivered(&self, sent: &Sent, via: &str) -> bool {
        self.clean()
            && self.handover.as_ref().is_some_and(|h| h.via == via)
            && handed_over(sent.state)
            && self.got(&sent.token)
            && self.answer().is_some()
    }
}

/// Whether a request of `stub` holds both `token` and `text`.
fn together(stub: &StubLog, token: &str, text: &str) -> bool {
    stub.requests
        .iter()
        .any(|body| body.contains(token) && body.contains(text))
}

/// The app-server path's checks: each returns its line in the output.
#[derive(Debug, Default)]
pub struct AppServerCheck {
    checks: Checks,
}

impl AppServerCheck {
    /// The idle thread the client started, whose message waited before it started.
    pub fn idle(&mut self, delivery: &Delivery, sent: &Sent) -> String {
        let pass = delivery.delivered(sent, "turn/start");
        let details = format!(
            "{}; {} the client started",
            delivery.summary(),
            delivery.delays(sent, delivery.started_ms)
        );
        self.checks.report("idle thread", pass, &details)
    }

    /// The busy thread and its message, added when the agent started the shell command in the
    /// thread's first turn, if it did: it goes into that turn.
    pub fn busy(&mut self, delivery: &Delivery, sent: Option<&Sent>) -> String {
        let Some(sent) = sent else {
            let details = format!("{}; the agent ran no shell command", delivery.summary());
            return self.checks.report("busy thread", false, &details);
        };
        let first = delivery.turns.first();
        let pass = delivery.delivered(sent, "turn/steer")
            && delivery.handover.as_ref().map(|h| &h.turn) == first;
        let details = format!(
            "{}; added while the shell command ran, {} adding",
            delivery.summary(),
            delivery.delays(sent, sent.added_at_ms)
        );
        self.checks.report("busy thread", pass, &details)
    }

    /// The other of two threads on one app-server, one the client started next to the session a
    /// person works in there: its message reaches its thread, not the person's, whose terminal UI
    /// `shown` it or not.
    pub fn other(&mut self, delivery: &Delivery, sent: &Sent, shown: bool) -> String {
        let in_person = shown
            || delivery
                .stub
                .as_ref()
                .is_some_and(|stub| together(stub, &sent.token, &person_prompt()));
        let pass = delivery.delivered(sent, "turn/start") && !in_person;
        let details = format!(
            "{}; {} the client started; the token {} the person's session",
            delivery.summary(),
            delivery.delays(sent, delivery.started_ms),
            if in_person {
                "reached"
            } else {
                "did not reach"
            }
        );
        self.checks
            .report("the other of two threads", pass, &details)
    }

    /// The thread of the session a person works in, which the client resumed: the message reaches
    /// the model in that thread, and the person's terminal UI `shown` it.
    pub fn person(&mut self, delivery: &Delivery, sent: &Sent, shown: bool) -> String {
        let in_thread = match &delivery.stub {
            Some(stub) => together(stub, &sent.token, &person_prompt()),
            None => delivery.got(&sent.token),
        };
        let pass = delivery.delivered(sent, "turn/start") && in_thread && shown;
        let details = format!(
            "{}; {} the client started; the token {} the session's thread, its terminal UI {} it",
            delivery.summary(),
            delivery.delays(sent, delivery.started_ms),
            if in_thread {
                "reached"
            } else {
                "did not reach"
            },
            if shown { "showed" } else { "did not show" }
        );
        self.checks
            .report("the session a person works in", pass, &details)
    }

    /// Whether every check passed.
    pub fn pass(&self) -> bool {
        self.checks.pass()
    }

    /// The last line of the output.
    pub fn verdict(&self, options: &Options) -> String {
        self.checks.verdict("app-server", Agent::Codex, options)
    }
}

#[cfg(test)]
mod tests {
    use agent_source::State;

    use super::*;

    #[test]
    fn an_app_server_loads_as_little_as_it_allows_and_runs_on_the_stub_or_the_model() {
        assert_eq!(
            options(Some("gpt-5.5"), None),
            [
                "--disable",
                "plugins",
                "-c",
                "notify=[]",
                "-c",
                r#"model="gpt-5.5""#,
                "-c",
                "skills.include_instructions=false",
                "-c",
                r#"model_reasoning_effort="low""#,
            ]
        );
        let stubbed = options(None, Some("http://127.0.0.1:4100"));
        assert!(
            !stubbed.iter().any(|arg| arg.starts_with("model=")),
            "{stubbed:?}"
        );
        assert!(
            stubbed.contains(
                &r#"model_providers.stub.base_url="http://127.0.0.1:4100/v1""#.to_owned()
            ),
            "{stubbed:?}"
        );
    }

    #[test]
    fn a_listening_app_server_trusts_the_project_of_the_terminal_ui() {
        // A `"` in the path must not end the TOML string.
        assert_eq!(
            listening(
                Path::new("/w/s.sock"),
                Path::new("/w/a\"b"),
                &["-c".to_owned(), "x=1".to_owned()]
            ),
            [
                "app-server",
                "--listen",
                "unix:///w/s.sock",
                "-c",
                "x=1",
                "-c",
                r#"projects={"/w/a\"b"={trust_level="trusted"}}"#,
            ]
        );
    }

    #[test]
    fn the_client_runs_its_own_app_server_or_connects_to_one() {
        let files = Files::new(Path::new("/w/a"));
        let own = Connect::Own(vec!["-c".to_owned(), "x=1".to_owned()]);
        assert_eq!(
            client(&files, &own, Some("Run it.")),
            [
                "/w/a/source",
                "--log",
                "/w/a/exchange.jsonl",
                "--prompt",
                "Run it.",
                "--",
                "-c",
                "x=1",
            ]
        );
        let socket = Path::new("/w/s.sock");
        let new = Connect::Socket {
            path: socket,
            thread: None,
        };
        assert_eq!(
            client(&files, &new, None),
            [
                "/w/a/source",
                "--log",
                "/w/a/exchange.jsonl",
                "--socket",
                "/w/s.sock"
            ]
        );
        let person = Connect::Socket {
            path: socket,
            thread: Some("t1"),
        };
        assert_eq!(
            client(&files, &person, None)[3..],
            ["--socket", "/w/s.sock", "--thread", "t1"]
        );
    }

    #[test]
    fn the_person_works_in_the_terminal_ui_connected_to_the_app_server() {
        assert_eq!(
            person(
                Path::new("/w/s.sock"),
                Path::new("/w/d/project"),
                Some("gpt-5.5")
            ),
            [
                "--remote",
                "unix:///w/s.sock",
                "-C",
                "/w/d/project",
                "-m",
                "gpt-5.5",
                "Reply with only OK.",
            ]
        );
        assert_eq!(
            person(Path::new("/w/s.sock"), Path::new("/w/d/project"), None).len(),
            5
        );
    }

    #[test]
    fn finds_the_persons_thread_once_its_first_turn_ended() {
        // Shortened from a `thread/list` result of Codex 0.162.1.
        let list = |status: &str| {
            json!({ "data": [
                { "id": "t0", "preview": "Generate a title", "status": { "type": "idle" } },
                { "id": "t1", "preview": "Reply with only OK.", "status": { "type": status } },
            ], "nextCursor": null })
        };
        assert_eq!(person_thread(&list("idle")).as_deref(), Some("t1"));
        assert_eq!(person_thread(&list("active")), None);
        assert_eq!(person_thread(&json!({ "data": [] })), None);
        assert_eq!(person_thread(&json!({})), None);
    }

    /// What a client printed: a turn that took the message and answered `reply`.
    fn delivered(via: &str, reply: &str) -> Delivery {
        let mut delivery = Delivery::new(1_000);
        for (line, at_ms) in [
            (r#"{"event":"thread","threadId":"t1"}"#.to_owned(), 1_100),
            (r#"{"event":"turnStarted","turnId":"u1"}"#.to_owned(), 1_200),
            (
                r#"{"event":"commandStarted","turnId":"u1"}"#.to_owned(),
                1_300,
            ),
            (
                r#"{"event":"commandStarted","turnId":"u1"}"#.to_owned(),
                1_350,
            ),
            (
                format!(r#"{{"event":"handedOver","via":"{via}","turnId":"u1","msgIds":["m1"]}}"#),
                1_400,
            ),
            (
                format!(
                    r#"{{"event":"reply","turnId":"u1","text":{}}}"#,
                    json!(reply)
                ),
                3_400,
            ),
            (
                r#"{"event":"tokens","input":10,"cached":4,"output":2}"#.to_owned(),
                3_450,
            ),
            (
                r#"{"event":"turnCompleted","turnId":"u1","status":"completed"}"#.to_owned(),
                3_500,
            ),
            ("not JSON".to_owned(), 3_600),
        ] {
            delivery.record(&line, at_ms);
        }
        delivery
    }

    fn sent(token: &str, state: Option<State>) -> Sent {
        Sent {
            token: token.to_owned(),
            added_at_ms: 1_200,
            state,
        }
    }

    fn handed() -> Option<State> {
        Some(State::HandedOver { at_ms: 1_400 })
    }

    fn on_stub(delivery: Delivery, bodies: &[&str], credentials: &[&str]) -> Delivery {
        Delivery {
            stub: Some(StubLog {
                requests: bodies.iter().map(|body| (*body).to_owned()).collect(),
                credentials: credentials.iter().map(|name| (*name).to_owned()).collect(),
            }),
            ..delivery
        }
    }

    #[test]
    fn reads_what_the_client_printed() {
        let mut delivery = delivered("turn/start", "tok-1");
        assert_eq!(delivery.thread.as_deref(), Some("t1"));
        assert_eq!(delivery.turns, ["u1"]);
        assert_eq!(delivery.completed, ["u1"]);
        assert_eq!(
            delivery.command_started_ms,
            Some(1_300),
            "the first command counts"
        );
        assert_eq!(
            delivery.handover,
            Some(Handover {
                via: "turn/start".to_owned(),
                turn: "u1".to_owned(),
                at_ms: 1_400
            })
        );
        assert_eq!(
            delivery.usage,
            Some(Usage {
                input: 10,
                cached: 4,
                output: 2
            })
        );
        assert_eq!(delivery.answer().map(|reply| reply.at_ms), Some(3_400));
        assert!(delivery.got("tok-1"));
        assert!(!delivery.got("tok-2"));
        for line in [
            r#"{"event":"refused","via":"turn/steer","error":"no active turn to steer"}"#,
            r#"{"event":"declined","method":"item/commandExecution/requestApproval"}"#,
            r#"{"event":"handedOver","via":"turn/start","turnId":"u2","msgIds":["m2"]}"#,
        ] {
            delivery.record(line, 4_000);
        }
        assert_eq!(delivery.refused, 1);
        assert_eq!(delivery.declined, ["item/commandExecution/requestApproval"]);
        assert_eq!(
            delivery.handover.as_ref().unwrap().turn,
            "u1",
            "the first handover counts"
        );
        assert_eq!(
            delivery.summary(),
            r#"replies ["tok-1"], refused 1 time(s), declined ["item/commandExecution/requestApproval"], tokens: input 10 (cached 4), output 2"#
        );
    }

    #[test]
    fn a_turn_answers_once_it_completed() {
        let mut delivery = Delivery::new(1_000);
        delivery.record(
            r#"{"event":"handedOver","via":"turn/start","turnId":"u1","msgIds":["m1"]}"#,
            1_100,
        );
        delivery.record(r#"{"event":"reply","turnId":"u1","text":"tok-1"}"#, 1_200);
        assert_eq!(delivery.answer(), None);
        assert!(
            !delivery.got("tok-1"),
            "on the real model the answer counts"
        );
        delivery.record(r#"{"event":"reply","turnId":"u0","text":"other"}"#, 1_250);
        delivery.record(r#"{"event":"turnCompleted","turnId":"u1"}"#, 1_300);
        assert_eq!(delivery.answer().map(|r| r.text.as_str()), Some("tok-1"));
        // A reply of another turn is not the answer.
        let mut other = Delivery::new(1_000);
        other.record(
            r#"{"event":"handedOver","via":"turn/start","turnId":"u1"}"#,
            1_100,
        );
        other.record(r#"{"event":"reply","turnId":"u0","text":"tok-1"}"#, 1_200);
        other.record(r#"{"event":"turnCompleted","turnId":"u1"}"#, 1_300);
        assert_eq!(other.answer(), None);
        assert_eq!(Delivery::new(0).answer(), None);
    }

    #[test]
    fn checks_the_idle_and_busy_threads() {
        let idle = on_stub(delivered("turn/start", "Done."), &["{\"tok-1\"}"], &[]);
        let mut check = AppServerCheck::default();
        assert_eq!(
            check.idle(&idle, &sent("tok-1", handed())),
            r#"  idle thread: pass — replies ["Done."], requests to the stub: 1; handed over in turn/start 0.4 s and answered 2.4 s after the client started"#
        );
        let busy = on_stub(delivered("turn/steer", "Done."), &["{\"tok-1\"}"], &[]);
        assert_eq!(
            check.busy(&busy, Some(&sent("tok-1", handed()))),
            r#"  busy thread: pass — replies ["Done."], requests to the stub: 1; added while the shell command ran, handed over in turn/steer 0.2 s and answered 2.2 s after adding"#
        );
        assert!(check.pass());
        assert_eq!(
            check.verdict(&Options::default()),
            "app-server on codex with the model stub: pass"
        );

        let mut other_turn = busy.clone();
        other_turn.turns.insert(0, "u0".to_owned());
        let mut declined = idle.clone();
        declined
            .declined
            .push("item/commandExecution/requestApproval".to_owned());
        for (delivery, sent, case) in [
            (
                idle.clone(),
                sent("tok-1", Some(State::Waiting)),
                "the message waits",
            ),
            (
                idle.clone(),
                sent("tok-2", handed()),
                "the token did not reach the model",
            ),
            (
                busy.clone(),
                sent("tok-1", handed()),
                "steered, not started",
            ),
            (declined, sent("tok-1", handed()), "an approval was asked"),
            (
                on_stub(
                    delivered("turn/start", "Done."),
                    &["{\"tok-1\"}"],
                    &["authorization"],
                ),
                sent("tok-1", handed()),
                "a credential other than the placeholder",
            ),
            (
                on_stub(Delivery::new(1_000), &["{\"tok-1\"}"], &[]),
                sent("tok-1", handed()),
                "never handed over",
            ),
        ] {
            let mut check = AppServerCheck::default();
            let line = check.idle(&delivery, &sent);
            assert!(line.contains(": fail — "), "{case}: {line}");
            assert!(!check.pass(), "{case}");
        }
        for (delivery, sent, case) in [
            (
                idle.clone(),
                Some(sent("tok-1", handed())),
                "started, not steered",
            ),
            (
                other_turn,
                Some(sent("tok-1", handed())),
                "steered into a later turn",
            ),
            (busy.clone(), None, "no shell command"),
        ] {
            let mut check = AppServerCheck::default();
            let line = check.busy(&delivery, sent.as_ref());
            assert!(line.contains(": fail — "), "{case}: {line}");
        }
    }

    #[test]
    fn on_the_real_model_the_answer_repeats_the_token() {
        let mut check = AppServerCheck::default();
        let line = check.idle(&delivered("turn/start", "tok-1"), &sent("tok-1", handed()));
        assert!(line.contains(": pass — "), "{line}");
        let line = check.idle(&delivered("turn/start", "OK"), &sent("tok-1", handed()));
        assert!(line.contains(": fail — "), "{line}");
        let real = Options {
            real: true,
            ..Options::default()
        };
        assert_eq!(
            check.verdict(&real),
            "app-server on codex with the real model: fail"
        );
    }

    #[test]
    fn checks_the_other_thread_and_the_persons_session() {
        // On one stub: the person's thread holds the person's prompt.
        let prompt = person_prompt();
        let bodies = [
            format!("{{\"{prompt}\"}}"),
            "{\"tok-b\"}".to_owned(),
            format!("{{\"{prompt}\", \"tok-d\"}}"),
        ];
        let bodies: Vec<&str> = bodies.iter().map(String::as_str).collect();
        let mut check = AppServerCheck::default();
        let b = on_stub(delivered("turn/start", "Done."), &bodies, &[]);
        assert_eq!(
            check.other(&b, &sent("tok-b", handed()), false),
            r#"  the other of two threads: pass — replies ["Done."], requests to the stub: 3; handed over in turn/start 0.4 s and answered 2.4 s after the client started; the token did not reach the person's session"#
        );
        assert_eq!(
            check.person(&b, &sent("tok-d", handed()), true),
            r#"  the session a person works in: pass — replies ["Done."], requests to the stub: 3; handed over in turn/start 0.4 s and answered 2.4 s after the client started; the token reached the session's thread, its terminal UI showed it"#
        );
        assert!(check.pass());

        for (pass, line) in [
            // The person's session got the other thread's message.
            (false, check.other(&b, &sent("tok-d", handed()), false)),
            (false, check.other(&b, &sent("tok-b", handed()), true)),
            (
                false,
                check.other(&b, &sent("tok-b", Some(State::Waiting)), false),
            ),
            // The message reached another thread, or the person did not see it.
            (false, check.person(&b, &sent("tok-b", handed()), true)),
            (false, check.person(&b, &sent("tok-d", handed()), false)),
            (
                false,
                check.person(&b, &sent("tok-d", Some(State::Taken)), true),
            ),
        ] {
            assert_eq!(line.contains(": pass — "), pass, "{line}");
        }

        // On the real model: the client's answer and the terminal UI.
        let real = delivered("turn/start", "tok-d");
        let mut check = AppServerCheck::default();
        assert!(
            check
                .person(&real, &sent("tok-d", handed()), true)
                .contains(": pass — ")
        );
        assert!(
            check
                .other(&real, &sent("tok-d", handed()), false)
                .contains(": pass — ")
        );
        assert!(
            check
                .other(&real, &sent("tok-d", handed()), true)
                .contains(": fail — ")
        );
        assert!(
            check
                .person(
                    &delivered("turn/start", "OK"),
                    &sent("tok-d", handed()),
                    true
                )
                .contains(": fail — ")
        );
    }
}
