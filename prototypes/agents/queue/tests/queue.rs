//! `agent-queue` against a stand-in for `codex`: a script that records each `codex queue` call and
//! which messages were handed over while it ran, and fails if told to.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use agent_source::{Message, Source, State};

/// A directory with the stand-in `codex` in `bin` and its records in `calls`.
struct StandIn {
    dir: tempfile::TempDir,
}

impl StandIn {
    /// The stand-in writes its arguments, one per line, to `calls/<n>/args`, and the names in the
    /// source's `handed` directory to `calls/<n>/handed`; it exits with the code in `exit`, if any.
    fn new(source: &Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(dir.path().join("calls")).unwrap();
        let script = format!(
            "#!/bin/sh\n\
             calls='{calls}'\n\
             call=\"$calls/$(ls \"$calls\" | wc -l | tr -d ' ')\"\n\
             mkdir \"$call\"\n\
             printf '%s\\n' \"$@\" >\"$call/args\"\n\
             ls '{handed}' >\"$call/handed\"\n\
             if [ -f '{exit}' ]; then echo 'thread not found' >&2; exit \"$(cat '{exit}')\"; fi\n",
            calls = dir.path().join("calls").display(),
            handed = source.join("handed").display(),
            exit = dir.path().join("exit").display(),
        );
        let codex = bin.join("codex");
        fs::write(&codex, script).unwrap();
        fs::set_permissions(&codex, fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    fn bin(&self) -> PathBuf {
        self.dir.path().join("bin")
    }

    /// Makes the stand-in exit with `code`.
    fn fail_with(&self, code: u8) {
        fs::write(self.dir.path().join("exit"), code.to_string()).unwrap();
    }

    /// Each call's arguments and the messages handed over while it ran.
    fn calls(&self) -> Vec<(Vec<String>, String)> {
        let calls = self.dir.path().join("calls");
        (0..fs::read_dir(&calls).unwrap().count())
            .map(|n| {
                let call = calls.join(n.to_string());
                let args = fs::read_to_string(call.join("args")).unwrap();
                (
                    args.lines().map(String::from).collect(),
                    fs::read_to_string(call.join("handed")).unwrap(),
                )
            })
            .collect()
    }
}

/// Runs `agent-queue <source> <thread>` with only `PATH` in its environment: `bin`, then the
/// system's for the stand-in's shell.
fn agent_queue(bin: &Path, source: &Path, thread: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-queue"))
        .arg(source)
        .arg(thread)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn state(source: &Source, message: &Message) -> Option<State> {
    source.state(&message.msg_id).unwrap()
}

/// Asserts that `text` carries each of `messages` once, with its sender label and `msg_id`.
fn carries_once(text: &str, messages: &[Message]) {
    for message in messages {
        let element = format!(
            "<message msg_id=\"{}\" from=\"{}\">\n{}\n</message>",
            message.msg_id, message.sender, message.text
        );
        assert_eq!(text.matches(&element).count(), 1, "{text}");
        assert_eq!(text.matches(&message.msg_id).count(), 1, "{text}");
    }
}

#[test]
fn queues_each_waiting_message_once_and_marks_it_after_the_command_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let codex = StandIn::new(dir.path());
    let messages = [
        source.add("alice of skald", "first").unwrap(),
        source.add("bob", "second").unwrap(),
    ];
    let output = agent_queue(&codex.bin(), dir.path(), "thread-1");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "2\n");
    let calls = codex.calls();
    assert_eq!(calls.len(), 1);
    let (args, handed) = &calls[0];
    // The message text is the last argument; it holds new lines.
    let head: Vec<&str> = args.iter().take(4).map(String::as_str).collect();
    assert_eq!(head, ["queue", "--thread", "thread-1", "--message"]);
    carries_once(&args[4..].join("\n"), &messages);
    assert_eq!(handed, "", "nothing was handed over while the command ran");
    for message in &messages {
        assert!(matches!(
            state(&source, message),
            Some(State::HandedOver { .. })
        ));
    }

    // Handed over: with none waiting, it runs no command.
    let output = agent_queue(&codex.bin(), dir.path(), "thread-1");
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "0\n");
    assert_eq!(codex.calls().len(), 1);

    let later = [source.add("carol", "third").unwrap()];
    assert!(
        agent_queue(&codex.bin(), dir.path(), "thread-2")
            .status
            .success()
    );
    let calls = codex.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0[2], "thread-2");
    let text = calls[1].0[4..].join("\n");
    carries_once(&text, &later);
    assert!(!text.contains(&messages[0].msg_id), "{text}");
}

#[test]
fn a_failed_command_leaves_the_messages_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let source = Source::open(dir.path()).unwrap();
    let codex = StandIn::new(dir.path());
    let message = source.add("alice", "hi").unwrap();
    codex.fail_with(1);
    let output = agent_queue(&codex.bin(), dir.path(), "thread-1");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert_eq!(
        stderr(&output),
        "agent-queue: codex queue failed with exit status: 1: thread not found\n"
    );
    assert!(output.stdout.is_empty());
    assert_eq!(codex.calls().len(), 1);
    assert_eq!(state(&source, &message), Some(State::Waiting));

    // Without the command at all.
    let empty = tempfile::tempdir().unwrap();
    let output = agent_queue(empty.path(), dir.path(), "thread-1");
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).starts_with("agent-queue: "),
        "{}",
        stderr(&output)
    );
    assert_eq!(state(&source, &message), Some(State::Waiting));

    // The next run that succeeds hands it over.
    fs::remove_file(codex.dir.path().join("exit")).unwrap();
    assert!(
        agent_queue(&codex.bin(), dir.path(), "thread-1")
            .status
            .success()
    );
    assert!(matches!(
        state(&source, &message),
        Some(State::HandedOver { .. })
    ));
}

#[test]
fn wrong_arguments_print_the_usage() {
    let file = tempfile::NamedTempFile::new().unwrap();
    for args in [&[][..], &["dir"], &["dir", "thread", "more"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-queue"))
            .args(args)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert_eq!(
            stderr(&output),
            "usage: agent-queue <source-dir> <thread>\n"
        );
    }
    let output = agent_queue(Path::new("/nonexistent"), file.path(), "thread-1");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("agent-queue: "),
        "{}",
        stderr(&output)
    );
}
