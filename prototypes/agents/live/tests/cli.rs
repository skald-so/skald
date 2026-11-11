//! `live-check`'s commands, without running an agent CLI.

use std::process::{Command, Output};

use agent_source::{Source, State};

fn live_check(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(args)
        .output()
        .expect("live-check runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn add_puts_a_waiting_message_in_the_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let source = source.to_str().unwrap();
    let output = live_check(&["add", source, "a sender", "a text"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let msg_id = String::from_utf8(output.stdout).unwrap();
    let msg_id = msg_id.trim();
    let source = Source::open(source).unwrap();
    assert_eq!(source.state(msg_id).unwrap(), Some(State::Waiting));
    let taken = source.take().unwrap();
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].msg_id, msg_id);
    assert_eq!(taken[0].sender, "a sender");
    assert_eq!(taken[0].text, "a text");
}

#[test]
fn add_fails_when_the_source_cannot_open() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let output = live_check(&["add", file.path().to_str().unwrap(), "s", "t"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("live-check: "),
        "{}",
        stderr(&output)
    );
}

#[test]
fn wrong_arguments_print_the_usage() {
    for args in [
        &[][..],
        &["add", "dir", "sender"],
        &["inbox"],
        &["inbox", "gemini"],
        &["inbox", "claude", "--model"],
        &["inbox", "codex", "--verbose"],
        &["inbox", "codex", "--all", "--real", "--stub"],
    ] {
        let output = live_check(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(stderr(&output).starts_with("usage: live-check"), "{args:?}");
    }
}

#[test]
fn inbox_fails_without_the_cli() {
    let empty = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_live-check"))
        .args(["inbox", "claude", "--all", "--real", "--keep"])
        .env("PATH", empty.path())
        .output()
        .expect("live-check runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).starts_with("live-check: "),
        "{}",
        stderr(&output)
    );
}
