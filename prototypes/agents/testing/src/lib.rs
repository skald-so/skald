//! Fixtures, assertions and recorded exchanges the delivery prototypes' tests share.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Output;
use std::thread;
use std::time::{Duration, Instant};

use agent_source::{Message, Source, State};
use rmcp::model::CustomNotification;
use rmcp::service::NotificationContext;
use rmcp::{ClientHandler, RoleClient};
use serde_json::Value;
use tempfile::TempDir;
use tokio::sync::mpsc;

/// A message `msg_id` from `alice` saying `hi`, never added to a source.
pub fn message(msg_id: &str) -> Message {
    Message {
        msg_id: msg_id.to_owned(),
        sender: "alice".to_owned(),
        text: "hi".to_owned(),
        added_at_ms: 1,
        handed_over_at_ms: None,
        ack: None,
    }
}

/// A source in a temporary directory.
pub fn opened() -> (TempDir, Source) {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    (dir, source)
}

/// Opens the source in `dir` and adds a message from `alice` saying `hi`.
pub fn one_waiting(dir: &Path) -> (Source, Message) {
    let source = Source::open(dir).unwrap();
    let message = source.add("alice", "hi").unwrap();
    (source, message)
}

/// Adds `count` messages from `alice`, `message 0` and on, to `source`; returns their `msg_id`s.
pub fn add_many(source: &Source, count: usize) -> Vec<String> {
    (0..count)
        .map(|i| source.add("alice", &format!("message {i}")).unwrap().msg_id)
        .collect()
}

/// Adds a message from `alice` saying `hi` to `source`, in `dir`, that a server took and died with
/// before handing it over: its claim stays.
pub fn taken_by_a_dead_server(dir: &Path, source: &Source) -> Message {
    let crashed = Source::open(dir).unwrap();
    let message = source.add("alice", "hi").unwrap();
    assert_eq!(crashed.take().unwrap(), vec![message.clone()]);
    drop(crashed);
    message
}

/// An MCP client that passes on each custom notification it gets, such as a channel's events.
pub struct Events(pub mpsc::UnboundedSender<CustomNotification>);

impl ClientHandler for Events {
    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) {
        let _ = self.0.send(notification);
    }
}

/// `pairs` as owned strings, such as variables or headers.
pub fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// Writes `script` to `path` as an executable file, such as a stand-in for a CLI.
pub fn write_executable(path: &Path, script: &str) {
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A command's stderr as text.
pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Asserts that a command failed with exit code 1 and an error that starts with `error`.
pub fn assert_failed(output: &Output, error: &str) {
    assert_eq!(output.status.code(), Some(1), "{}", stderr(output));
    assert!(stderr(output).starts_with(error), "{}", stderr(output));
}

/// Asserts that `text` carries each of `messages` once: its element with its `msg_id`, sender label
/// and text, and its `msg_id` nowhere else.
pub fn assert_carries_once(text: &str, messages: &[Message]) {
    for message in messages {
        let element = format!(
            "<message msg_id=\"{}\" from=\"{}\">\n{}\n</message>",
            message.msg_id, message.sender, message.text
        );
        assert_eq!(text.matches(&element).count(), 1, "{text}");
        assert_eq!(text.matches(&message.msg_id).count(), 1, "{text}");
    }
}

/// Whether `source` marks the message `msg_id` handed over.
pub fn handed_over(source: &Source, msg_id: &str) -> bool {
    matches!(
        source.state(msg_id).unwrap(),
        Some(State::HandedOver { .. })
    )
}

/// Asserts that `source` marks each of `messages` handed over.
pub fn assert_handed_over(source: &Source, messages: &[Message]) {
    for message in messages {
        assert!(handed_over(source, &message.msg_id), "{}", message.msg_id);
    }
}

/// Asserts that each of `messages` waits in `source`.
pub fn assert_waiting(source: &Source, messages: &[Message]) {
    for message in messages {
        assert_eq!(
            source.state(&message.msg_id).unwrap(),
            Some(State::Waiting),
            "{}",
            message.msg_id
        );
    }
}

/// Waits up to 10 s until `source` marks `msg_id` handed over: a server marks a message after it
/// writes it to the CLI.
pub fn wait_handed_over(source: &Source, msg_id: &str) {
    let start = Instant::now();
    while !handed_over(source, msg_id) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{msg_id} is not marked handed over: {:?}",
            source.state(msg_id).unwrap()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// An exchange recorded from the Codex app-server: each line `{"client": <message>}` or
/// `{"server": <message>}`.
pub struct Exchange(pub Vec<Value>);

impl Exchange {
    /// The exchange in the file at `path`, one line each.
    pub fn read(path: &Path) -> Self {
        let text = fs::read_to_string(path).unwrap();
        Self(
            text.lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
        )
    }

    /// The exchange up to the first line for which `last` holds, that line included.
    pub fn until(mut self, last: impl Fn(&Value) -> bool) -> Self {
        let end = self.0.iter().position(last).unwrap();
        self.0.truncate(end + 1);
        self
    }

    /// The client's messages.
    pub fn client(&self) -> Vec<&Value> {
        self.0
            .iter()
            .filter_map(|line| line.get("client"))
            .collect()
    }

    /// The server's messages after each of the client's, before its next one.
    pub fn answers(&self) -> Vec<Vec<&Value>> {
        let mut answers: Vec<Vec<&Value>> = Vec::new();
        for line in &self.0 {
            match (line.get("client"), line.get("server")) {
                (Some(_), _) => answers.push(Vec::new()),
                (_, Some(message)) => answers.last_mut().unwrap().push(message),
                _ => panic!("{line}"),
            }
        }
        answers
    }

    /// Puts in `dir/bin` a stand-in `codex` that replays the exchange as `codex app-server` on
    /// stdio. It writes its arguments, one per line, to `dir/args`; then for each line it reads, it
    /// writes the line to `dir/received/<n>`, runs the shell commands `on_line`, and answers with
    /// the server's messages that follow the client's `n`-th in the exchange, kept in
    /// `dir/answers/<n>`. Once its stdin ends, it runs `at_end`. Both commands find `dir` in `$dir`
    /// and the line's number in `$n`.
    pub fn stand_in(&self, dir: &Path, on_line: &str, at_end: &str) {
        for sub in ["bin", "received", "answers"] {
            fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for (n, answer) in self.answers().iter().enumerate() {
            let lines: String = answer
                .iter()
                .map(|message| format!("{message}\n"))
                .collect();
            fs::write(dir.join(format!("answers/{n}")), lines).unwrap();
        }
        let script = format!(
            "#!/bin/sh\n\
             dir='{dir}'\n\
             printf '%s\\n' \"$@\" >\"$dir/args\"\n\
             n=0\n\
             while IFS= read -r line; do\n\
             printf '%s\\n' \"$line\" >\"$dir/received/$n\"\n\
             {on_line}\n\
             if [ -f \"$dir/answers/$n\" ]; then cat \"$dir/answers/$n\"; fi\n\
             n=$((n + 1))\n\
             done\n\
             {at_end}\n",
            dir = dir.display()
        );
        write_executable(&dir.join("bin/codex"), &script);
    }

    /// The thread's ID, from the result of `thread/start` or `thread/resume`.
    pub fn thread(&self) -> String {
        self.0
            .iter()
            .find_map(|line| line["server"]["result"]["thread"]["id"].as_str())
            .unwrap()
            .to_owned()
    }
}
