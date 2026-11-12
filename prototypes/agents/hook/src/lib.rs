//! A hook handler for Claude Code and Codex that hands the waiting messages of a [`Source`] to the
//! agent:
//! - after a tool call (`PostToolUse`), it adds them to the agent's context next to the tool result;
//! - at the end of a turn (`Stop`), it keeps the turn going and gives them to the agent;
//! - on Codex, with the user's next prompt (`UserPromptSubmit`), it adds them to the context.
//!
//! With no message waiting it prints nothing, so `Stop` lets the turn end. It marks the messages
//! handed over once its output is written to the CLI and puts them back if the write fails, as
//! `docs/protocol/delivery.md`, "Device to agent", says.
//!
//! Each of these events also confirms the handed-over messages it finds in the session's own file,
//! the hook input's `transcript_path`, as the source's `transcript` signal; a waiting message
//! already there, which another path delivered first, it drops as a repeat by its `msg_id`. On
//! `SessionStart` it registers the session in the source: its ID, file and directory and, on Claude
//! Code, its inbox socket and token from the environment ([`Messaging`]).

use std::io::{self, ErrorKind, Write};
use std::path::Path;

use agent_inbox::render;
use agent_source::{Message, Registration, Source, now_ms};
use agent_transcript::read_seen;
use serde_json::{Value, json};

/// The CLI that runs the handler: their hooks' outputs differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    /// Whether the handler answers `event` on this CLI: Claude Code's `PostToolUse`, `Stop` and
    /// `SessionStart`; Codex's also `UserPromptSubmit`.
    pub fn answers(self, event: Event) -> bool {
        self == Self::Codex || event != Event::UserPromptSubmit
    }

    fn transcript(self) -> agent_transcript::Agent {
        match self {
            Self::Claude => agent_transcript::Agent::Claude,
            Self::Codex => agent_transcript::Agent::Codex,
        }
    }
}

/// What Claude Code exports to its hooks for cross-session messaging, read from the environment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Messaging {
    /// `CLAUDE_CODE_MESSAGING_SOCKET`: the session's inbox socket.
    pub socket: Option<String>,
    /// `CLAUDE_CODE_MESSAGING_TOKEN`: the session's token for it.
    pub token: Option<String>,
}

impl Messaging {
    /// The variables in the handler's environment.
    pub fn from_env() -> Self {
        let var = |name| std::env::var(name).ok().filter(|value| !value.is_empty());
        Self {
            socket: var("CLAUDE_CODE_MESSAGING_SOCKET"),
            token: var("CLAUDE_CODE_MESSAGING_TOKEN"),
        }
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
    /// The session started: the handler only registers it.
    SessionStart,
}

impl Event {
    /// The event of the hook input `input`, from its `hook_event_name`; `None` for any other.
    pub fn of(input: &Value) -> Option<Self> {
        match input["hook_event_name"].as_str()? {
            "PostToolUse" => Some(Self::PostToolUse),
            "Stop" => Some(Self::Stop),
            "UserPromptSubmit" => Some(Self::UserPromptSubmit),
            "SessionStart" => Some(Self::SessionStart),
            _ => None,
        }
    }

    /// The event's name in the hook input and output.
    pub fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::Stop => "Stop",
            Self::UserPromptSubmit => "UserPromptSubmit",
            Self::SessionStart => "SessionStart",
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

/// Handles the hook input `input` of `agent`, whose environment gave `messaging`: on
/// `SessionStart`, registers the session; on another event it answers, confirms the messages the
/// session's file shows, takes the waiting messages of `source`, drops those the file shows as
/// repeats, writes the output for the rest to `out` and marks them handed over; puts them back if
/// the write fails. Takes nothing for an event it does not answer.
pub fn handle(
    agent: Agent,
    input: &[u8],
    source: &Source,
    out: &mut impl Write,
    messaging: &Messaging,
) -> io::Result<()> {
    let input: Value = serde_json::from_slice(input)
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    let Some(event) = Event::of(&input).filter(|event| agent.answers(*event)) else {
        return Ok(());
    };
    let text = |key: &str| input[key].as_str().map(String::from);
    if event == Event::SessionStart {
        let session_id = text("session_id")
            .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "no session_id"))?;
        return source.register(&Registration {
            session_id,
            transcript_path: text("transcript_path"),
            cwd: text("cwd"),
            socket: messaging.socket.clone(),
            token: messaging.token.clone(),
            at_ms: now_ms(),
        });
    }
    let transcript = text("transcript_path");
    let transcript = transcript.as_deref().map(Path::new);
    if let Some(transcript) = transcript {
        confirm(agent, transcript, source)?;
    }
    let taken = source.take()?;
    if taken.is_empty() {
        return Ok(());
    }
    let ids = |messages: &[&Message]| -> Vec<String> {
        messages.iter().map(|m| m.msg_id.clone()).collect()
    };
    let seen: Vec<String> = match transcript {
        Some(transcript) => {
            let all: Vec<&Message> = taken.iter().collect();
            read_seen(agent.transcript(), transcript, &ids(&all))?
                .into_iter()
                .map(|seen| seen.msg_id)
                .collect()
        }
        None => Vec::new(),
    };
    let (repeats, fresh): (Vec<&Message>, Vec<&Message>) =
        taken.iter().partition(|m| seen.contains(&m.msg_id));
    if !repeats.is_empty() {
        source.handed_over(&ids(&repeats))?;
        for message in &repeats {
            source.record(&message.msg_id, "transcript", None)?;
            source.record(&message.msg_id, "repeat-dropped", Some(event.name()))?;
        }
    }
    if fresh.is_empty() {
        return Ok(());
    }
    let fresh_ids = ids(&fresh);
    let fresh: Vec<Message> = fresh.into_iter().cloned().collect();
    match writeln!(out, "{}", output(agent, event, &fresh)).and_then(|()| out.flush()) {
        Ok(()) => source.handed_over(&fresh_ids),
        Err(error) => {
            source.release(&fresh_ids)?;
            Err(error)
        }
    }
}

/// Records the `transcript` signal of each handed-over message of `source` that the session file
/// `transcript` shows reached the model; returns how many it found.
pub fn confirm(agent: Agent, transcript: &Path, source: &Source) -> io::Result<usize> {
    let handed: Vec<String> = source
        .handed_messages()?
        .into_iter()
        .map(|message| message.msg_id)
        .collect();
    let seen = read_seen(agent.transcript(), transcript, &handed)?;
    for found in &seen {
        let detail = found.at_ms.map(|at| format!("entry at {at}"));
        source.record(&found.msg_id, "transcript", detail.as_deref())?;
    }
    Ok(seen.len())
}

/// What `--take` prints for `messages`, at least one: their IDs and the text that hands them over.
pub fn taken(messages: &[Message]) -> Value {
    let ids: Vec<&str> = messages.iter().map(|m| m.msg_id.as_str()).collect();
    json!({ "ids": ids, "text": render(messages) })
}

#[cfg(test)]
mod tests {
    use agent_source::State;
    use agent_testing::message;

    use super::*;

    #[test]
    fn reads_the_event_of_a_hook_input() {
        let event = |name: &str| Event::of(&json!({ "hook_event_name": name }));
        assert_eq!(event("SessionStart"), Some(Event::SessionStart));
        assert_eq!(Event::SessionStart.name(), "SessionStart");
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
        assert!(Agent::Claude.answers(Event::SessionStart));
        assert!(Agent::Codex.answers(Event::SessionStart));
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
                let error = handle(
                    agent,
                    input,
                    &source,
                    &mut Broken { flush_only },
                    &Messaging::default(),
                )
                .unwrap_err();
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
        let error =
            handle(Agent::Codex, b"{", &source, &mut out, &Messaging::default()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(out.is_empty());
        assert_eq!(source.state(&added.msg_id).unwrap(), Some(State::Waiting));
    }
}
