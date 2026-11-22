//! Claude Code sessions waiting on their own permission prompt (`docs/dev/architecture.md`,
//! "Notifications"; `docs/dev/agent-delivery.md`, "Permission prompts"), for the person's
//! notification:
//!
//! - `skald hook claude` records that a session of an agent in a space waits on its permission
//!   prompt ([`told`]): on `PermissionRequest`, at once, and on `Notification` of type
//!   `permission_prompt`, about 6 s later. The record names the agent, its space, the session and
//!   the time, nothing of the prompt.
//! - Each prompt is recorded once: a `Notification` while the session's prompt of the same turn is
//!   open records nothing. Its `PostToolUse`, `Stop` and `SessionEnd` end it ([`ended`]).
//! - The person's `skald recv` shows each record once, as one line, or with `--json` an object of
//!   every field ([`show_to_person`]); an agent's output never does. Records go after [`KEPT`].
//! - These hooks print nothing.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use skald_proto::Name;

use crate::devices::text;
use crate::forms::{self, Form};
use crate::output::{self, Stream};
use crate::state::{self, PermissionPrompt, State};
use crate::{folders, hook};

/// How long the device keeps a record.
pub const KEPT: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The `type` of a record in `--json`.
pub const RECORD_TYPE: &str = "permission_prompt";

/// Records that the session `input` names, of `agent` in the working directory's space, waits on
/// its permission prompt, which a hook told on `event`; in a folder in no space, or without the
/// session's ID, nothing.
pub fn told(dir: &Path, agent: &Name, input: &hook::Input) -> Result<(), String> {
    let by_request = match input.event {
        Some(hook::Event::PermissionRequest) => true,
        Some(hook::Event::PermissionPrompt) => false,
        _ => return Ok(()),
    };
    let Some(session) = &input.session.id else {
        return Ok(());
    };
    let Ok(space) = folders::working_space(dir) else {
        return Ok(());
    };
    let now = state::unix_now();
    let prompt = PermissionPrompt {
        position: 0,
        space: space.id,
        agent: agent.as_str().to_owned(),
        session: session.clone(),
        at: now,
    };
    let state = State::open(dir).map_err(text)?;
    (state.prompt_told(&prompt, input.turn.as_deref(), by_request, kept_since(now)))
        .map(drop)
        .map_err(text)
}

/// The earliest time, in Unix seconds, of a record kept at `now`.
fn kept_since(now: u64) -> u64 {
    now.saturating_sub(KEPT.as_secs())
}

/// Records that the session of the ID `session`, if given, no longer waits on a prompt it was
/// recorded waiting on.
pub fn ended(dir: &Path, session: Option<&str>) -> Result<(), String> {
    match session {
        Some(id) => (State::open(dir).and_then(|state| state.prompts_ended(id))).map_err(text),
        None => Ok(()),
    }
}

/// Shows each record the person's run has yet to show, in `form`, in order, and marks each shown;
/// gives how many it showed.
pub fn show_to_person(state: &State, form: Form) -> Result<usize, String> {
    let prompts = state.prompts_for_person().map_err(text)?;
    if prompts.is_empty() {
        return Ok(0);
    }
    let spaces = state.spaces().map_err(text)?;
    for prompt in &prompts {
        let space = (spaces.iter())
            .find(|space| space.id == prompt.space)
            .map_or_else(|| prompt.space.to_string(), |space| space.name.clone());
        let line = shown(prompt, &space, form);
        output::write(Stream::Out, format!("{line}\n").as_bytes())
            .map_err(|err| format!("cannot write a permission prompt to the output: {err}"))?;
        state.prompt_shown(prompt.position).map_err(text)?;
    }
    Ok(prompts.len())
}

/// The line of `prompt`, of an agent in the space named `space`, in `form`.
fn shown(prompt: &PermissionPrompt, space: &str, form: Form) -> String {
    match form {
        Form::Json => forms::object(
            RECORD_TYPE,
            json!({
                "agent": prompt.agent,
                "space": space,
                "session": prompt.session,
                "timestamp": prompt.at,
            }),
        ),
        Form::Full | Form::Short => format!(
            "The agent {} in the space {space} waits on its permission prompt",
            prompt.agent
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skald_proto::SpaceId;

    fn prompt(session: &str, at: u64) -> PermissionPrompt {
        PermissionPrompt {
            position: 0,
            space: "0190b6a2-1f3c-7d4e-8a5b-6c7d8e9f0a1b"
                .parse::<SpaceId>()
                .unwrap(),
            agent: "web".to_owned(),
            session: session.to_owned(),
            at,
        }
    }

    fn sessions(state: &State) -> Vec<String> {
        let prompts = state.prompts_for_person().unwrap();
        prompts.into_iter().map(|prompt| prompt.session).collect()
    }

    /// A `PermissionRequest` always records a prompt; a `Notification` records one only while no
    /// prompt of the session and turn is open; the session's other hooks end it.
    #[test]
    fn each_prompt_is_recorded_once() {
        let state = State::in_memory().unwrap();
        let told = |session, turn, by_request| {
            (state.prompt_told(&prompt(session, 100), turn, by_request, 0)).unwrap()
        };
        assert!(told("a", Some("t1"), true));
        assert!(!told("a", Some("t1"), false), "told again");
        assert!(told("b", Some("t1"), false), "another session's");
        assert!(told("a", Some("t2"), false), "another turn's");
        assert!(!told("a", Some("t2"), false));
        state.prompts_ended("a").unwrap();
        assert!(told("a", Some("t2"), false), "after it ended");
        assert!(told("c", None, false));
        assert!(!told("c", None, false), "without a turn");
        assert!(told("c", None, true));
        assert_eq!(sessions(&state), ["a", "b", "a", "a", "c", "c"]);
    }

    /// The person's run shows each record once; a record goes once it is older than the kept time.
    #[test]
    fn records_are_shown_once_and_go_after_their_time() {
        let state = State::in_memory().unwrap();
        state.prompt_told(&prompt("a", 100), None, true, 0).unwrap();
        state.prompt_told(&prompt("b", 200), None, true, 0).unwrap();
        let first = state.prompts_for_person().unwrap()[0].clone();
        assert_eq!((first.agent.as_str(), first.at), ("web", 100));
        state.prompt_shown(first.position).unwrap();
        assert_eq!(sessions(&state), ["b"]);
        state
            .prompt_told(&prompt("c", 300), None, true, 200)
            .unwrap();
        assert_eq!(sessions(&state), ["b", "c"]);
        state
            .prompt_told(&prompt("d", 400), None, true, 201)
            .unwrap();
        assert_eq!(sessions(&state), ["c", "d"]);
    }

    /// A record is kept 7 days and goes after.
    #[test]
    fn records_are_kept_seven_days() {
        let state = State::in_memory().unwrap();
        let day = 24 * 60 * 60;
        let told = |session, at| {
            (state.prompt_told(&prompt(session, at), None, true, kept_since(at))).unwrap();
        };
        told("a", 1_000 * day);
        told("b", 1_007 * day);
        assert_eq!(sessions(&state), ["a", "b"]);
        told("c", 1_007 * day + 1);
        assert_eq!(sessions(&state), ["b", "c"]);
    }

    /// The person's line names the agent and its space; the JSON object holds every field.
    #[test]
    fn a_record_shows_its_agent_space_session_and_time() {
        let record = prompt("9428", 1_700);
        let line = shown(&record, "clienta", Form::Full);
        assert_eq!(
            line,
            "The agent web in the space clienta waits on its permission prompt"
        );
        let object: serde_json::Value =
            serde_json::from_str(&shown(&record, "clienta", Form::Json)).unwrap();
        assert_eq!(
            object,
            json!({
                "type": RECORD_TYPE,
                "agent": "web",
                "space": "clienta",
                "session": "9428",
                "timestamp": 1_700,
            })
        );
    }
}
