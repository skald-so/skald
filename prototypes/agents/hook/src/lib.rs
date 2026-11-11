//! A Claude Code hook handler that hands the waiting messages of a [`Source`] to the agent:
//! - after a tool call (`PostToolUse`), it adds them to the agent's context next to the tool result;
//! - at the end of a turn (`Stop`), it keeps the turn going and gives them to the agent.
//!
//! With no message waiting it prints nothing, so `Stop` lets the turn end. It marks the messages
//! handed over once its output is written to the CLI and puts them back if the write fails, as
//! `docs/protocol/delivery.md`, "Device to agent", says.

use std::io::{self, ErrorKind, Write};

use agent_inbox::render;
use agent_source::{Message, Source};
use serde_json::{Value, json};

/// A hook event the handler answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A tool call ended.
    PostToolUse,
    /// The agent's turn is about to end.
    Stop,
}

impl Event {
    /// The event of the hook input `input`, from its `hook_event_name`; `None` for any other.
    pub fn of(input: &Value) -> Option<Self> {
        match input["hook_event_name"].as_str()? {
            "PostToolUse" => Some(Self::PostToolUse),
            "Stop" => Some(Self::Stop),
            _ => None,
        }
    }

    /// The event's name in Claude Code's hook input and output.
    pub fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
        }
    }
}

/// The handler's output for `event` with `messages`; `None` with no messages: the handler prints
/// nothing, so the tool result goes on alone and `Stop` lets the turn end.
///
/// Both events add the messages as `additionalContext`. On `Stop` it keeps the turn going: Claude
/// Code labels it "Stop hook feedback", where `decision: "block"` would show a hook error.
pub fn output(event: Event, messages: &[Message]) -> Option<Value> {
    (!messages.is_empty()).then(|| {
        json!({
            "hookSpecificOutput": {
                "hookEventName": event.name(),
                "additionalContext": render(messages),
            }
        })
    })
}

/// Handles the hook input `input`: takes the waiting messages of `source`, writes the output to
/// `out` and marks them handed over; puts them back if the write fails. Takes nothing for an event
/// it does not answer.
pub fn handle(input: &[u8], source: &Source, out: &mut impl Write) -> io::Result<()> {
    let input: Value = serde_json::from_slice(input)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let Some(event) = Event::of(&input) else {
        return Ok(());
    };
    let messages = source.take()?;
    let Some(output) = output(event, &messages) else {
        return Ok(());
    };
    let ids: Vec<String> = messages.iter().map(|m| m.msg_id.clone()).collect();
    let written = writeln!(out, "{output}").and_then(|()| out.flush());
    if let Err(error) = written {
        source.release(&ids)?;
        return Err(error);
    }
    source.handed_over(&ids)
}

#[cfg(test)]
mod tests {
    use agent_source::State;

    use super::*;

    #[test]
    fn reads_the_event_of_a_hook_input() {
        let event = |name: &str| Event::of(&json!({ "hook_event_name": name }));
        assert_eq!(event("PostToolUse"), Some(Event::PostToolUse));
        assert_eq!(event("Stop"), Some(Event::Stop));
        assert_eq!(event("SubagentStop"), None);
        assert_eq!(event("PreToolUse"), None);
        assert_eq!(Event::of(&json!({})), None);
        assert_eq!(Event::of(&json!({ "hook_event_name": 1 })), None);
        assert_eq!(Event::PostToolUse.name(), "PostToolUse");
        assert_eq!(Event::Stop.name(), "Stop");
    }

    fn message(msg_id: &str) -> Message {
        Message {
            msg_id: msg_id.to_owned(),
            sender: "alice".to_owned(),
            text: "hi".to_owned(),
            added_at_ms: 1,
            handed_over_at_ms: None,
        }
    }

    #[test]
    fn adds_the_messages_as_context_of_the_event() {
        let messages = [message("m1"), message("m2")];
        for event in [Event::PostToolUse, Event::Stop] {
            assert_eq!(
                output(event, &messages),
                Some(json!({ "hookSpecificOutput": {
                    "hookEventName": event.name(),
                    "additionalContext": render(&messages),
                } }))
            );
            assert_eq!(output(event, &[]), None);
        }
    }

    /// A writer whose writes or flushes fail.
    struct Broken {
        flush_only: bool,
    }

    impl Write for Broken {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.flush_only {
                Ok(buf.len())
            } else {
                Err(io::Error::other("closed"))
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("closed"))
        }
    }

    #[test]
    fn a_failed_write_puts_the_messages_back() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let added = source.add("alice", "hi").unwrap();
        let input = br#"{"hook_event_name":"Stop"}"#;
        for flush_only in [false, true] {
            let error = handle(input, &source, &mut Broken { flush_only }).unwrap_err();
            assert_eq!(error.to_string(), "closed");
            assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
        }
    }

    #[test]
    fn input_that_is_not_json_is_an_error_and_takes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let added = source.add("alice", "hi").unwrap();
        let mut out = Vec::new();
        let error = handle(b"{", &source, &mut out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(out.is_empty());
        assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
    }
}
