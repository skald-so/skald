//! Assertions the delivery prototypes' tests share.

use std::process::Output;
use std::thread;
use std::time::{Duration, Instant};

use agent_source::{Message, Source, State};

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
