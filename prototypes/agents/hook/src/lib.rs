//! A hook handler for Claude Code and Codex that hands the waiting messages of a [`Source`] to the
//! agent:
//! - after a tool call (`PostToolUse`), it adds them to the agent's context next to the tool result;
//! - at the end of a turn (`Stop`), it keeps the turn going and gives them to the agent;
//! - on Codex, with the user's next prompt (`UserPromptSubmit`), it adds them to the context.
//!
//! With no message waiting it prints nothing, so `Stop` lets the turn end. It marks the messages
//! handed over once its output is written to the CLI and puts them back if the write fails, as
//! `docs/protocol/delivery.md`, "Device to agent", says.

use std::io::{self, ErrorKind, Write};

use agent_inbox::render;
use agent_source::{Message, Source};
use serde_json::{Value, json};

/// The CLI that runs the handler: their hooks' outputs differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    /// Whether the handler answers `event` on this CLI: Claude Code's `PostToolUse` and `Stop`;
    /// Codex's also `UserPromptSubmit`.
    pub fn answers(self, event: Event) -> bool {
        self == Self::Codex || event != Event::UserPromptSubmit
    }
}

/// A hook event the handler answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A tool call ended.
    PostToolUse,
    /// The agent's turn is about to end.
    Stop,
    /// The user submitted a prompt; the turn starts with it.
    UserPromptSubmit,
}

impl Event {
    /// The event of the hook input `input`, from its `hook_event_name`; `None` for any other.
    pub fn of(input: &Value) -> Option<Self> {
        match input["hook_event_name"].as_str()? {
            "PostToolUse" => Some(Self::PostToolUse),
            "Stop" => Some(Self::Stop),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            _ => None,
        }
    }

    /// The event's name in the hook input and output.
    pub fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
            Self::UserPromptSubmit => "UserPromptSubmit",
        }
    }
}

/// The handler's output on `agent` for `event` with `messages`, at least one.
///
/// Each event adds the messages as `additionalContext`, but Codex's `Stop`: there the turn goes on
/// with a continuation prompt, `decision: "block"` with the messages as its `reason`, and Codex
/// drops `additionalContext`. On Claude Code's `Stop`, context keeps the turn going: Claude Code
/// labels it "Stop hook feedback", where `decision: "block"` would show a hook error.
pub fn output(agent: Agent, event: Event, messages: &[Message]) -> Value {
    let text = render(messages);
    match (agent, event) {
        (Agent::Codex, Event::Stop) => json!({ "decision": "block", "reason": text }),
        _ => json!({
            "hookSpecificOutput": { "hookEventName": event.name(), "additionalContext": text }
        }),
    }
}

/// Handles the hook input `input` of `agent`: takes the waiting messages of `source`, writes the
/// output to `out` and marks them handed over; puts them back if the write fails. Takes nothing
/// for an event it does not answer.
pub fn handle(agent: Agent, input: &[u8], source: &Source, out: &mut impl Write) -> io::Result<()> {
    let input: Value = serde_json::from_slice(input)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let Some(event) = Event::of(&input).filter(|event| agent.answers(*event)) else {
        return Ok(());
    };
    source.hand_over(|messages| {
        writeln!(out, "{}", output(agent, event, messages)).and_then(|()| out.flush())
    })?;
    Ok(())
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
        assert_eq!(event("UserPromptSubmit"), Some(Event::UserPromptSubmit));
        assert_eq!(event("SubagentStop"), None);
        assert_eq!(event("PreToolUse"), None);
        assert_eq!(Event::of(&json!({})), None);
        assert_eq!(Event::of(&json!({ "hook_event_name": 1 })), None);
        assert_eq!(Event::PostToolUse.name(), "PostToolUse");
        assert_eq!(Event::Stop.name(), "Stop");
        assert_eq!(Event::UserPromptSubmit.name(), "UserPromptSubmit");
    }

    #[test]
    fn answers_user_prompt_submit_only_on_codex() {
        for event in [Event::PostToolUse, Event::Stop] {
            assert!(Agent::Claude.answers(event));
            assert!(Agent::Codex.answers(event));
        }
        assert!(!Agent::Claude.answers(Event::UserPromptSubmit));
        assert!(Agent::Codex.answers(Event::UserPromptSubmit));
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
        let context = |event: Event| {
            json!({ "hookSpecificOutput": {
                "hookEventName": event.name(),
                "additionalContext": render(&messages),
            } })
        };
        for event in [Event::PostToolUse, Event::Stop] {
            assert_eq!(output(Agent::Claude, event, &messages), context(event));
        }
        for event in [Event::PostToolUse, Event::UserPromptSubmit] {
            assert_eq!(output(Agent::Codex, event, &messages), context(event));
        }
    }

    #[test]
    fn on_codex_stop_continues_the_turn_with_the_messages() {
        let messages = [message("m1")];
        assert_eq!(
            output(Agent::Codex, Event::Stop, &messages),
            json!({ "decision": "block", "reason": render(&messages) })
        );
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
        for agent in [Agent::Claude, Agent::Codex] {
            for flush_only in [false, true] {
                let error = handle(agent, input, &source, &mut Broken { flush_only }).unwrap_err();
                assert_eq!(error.to_string(), "closed");
                assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
            }
        }
    }

    #[test]
    fn input_that_is_not_json_is_an_error_and_takes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let source = Source::open(dir.path()).unwrap();
        let added = source.add("alice", "hi").unwrap();
        let mut out = Vec::new();
        let error = handle(Agent::Codex, b"{", &source, &mut out).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(out.is_empty());
        assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
    }
}
