//! What the tests share: the hook inputs of a session that stops on its permission prompt, as
//! Claude Code gives them, and what the person's and the agents' runs show of a recorded prompt.

use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::hooks::Event;
use crate::kit::RECV;
use crate::lines::JSON_FLAG;
use crate::machine::Machine;
use crate::prompt_lines::{PERMISSION_WORD, agent_recv_shown, permission_lines};
use crate::registered::Ada;
use crate::senders::holds_word;
use crate::standin::Standin;
use crate::world::run;

/// Hook inputs recorded from Claude Code 2.1.328 for a `Bash` call nothing allowed, with the
/// paths of a temporary directory: the `PermissionRequest` hook's, and the `Notification` hook's
/// of type `permission_prompt`, the same prompt's.
const PERMISSION_REQUEST: &str = include_str!("recorded/permission-request.json");
const NOTIFICATION: &str = include_str!("recorded/notification.json");

/// The `type` of a recorded prompt in `--json`.
pub const RECORD_TYPE: &str = "permission_prompt";

/// How soon the person's `skald recv` shows a prompt a hook told.
pub const DETECTED: Duration = Duration::from_secs(5);

/// How long a test watches for a record that must not come.
pub const QUIET: Duration = Duration::from_secs(5);

/// The tool of every call that stops on the prompt.
pub const TOOL: &str = "Bash";

/// A tool call that stops on the prompt: its command and the model's description of it, neither of
/// which any output may hold.
#[derive(Debug, Clone)]
pub struct Call {
    pub command: String,
    pub description: String,
}

impl Call {
    /// The call that `name` marks.
    pub fn new(name: &str) -> Self {
        Call {
            command: format!("touch prompted-{name}.txt"),
            description: format!("Create the prompted-{name} file of the test"),
        }
    }

    /// The call's `tool_input`.
    fn input(&self) -> Value {
        json!({ "command": self.command, "description": self.description })
    }

    /// Fails if `shown` holds the call's command or description, or the tool's name.
    #[track_caller]
    pub fn assert_absent(&self, shown: &str, what: &str) {
        for part in [self.command.as_str(), self.description.as_str(), TOOL] {
            assert!(!shown.contains(part), "{what} holds {part:?}: {shown}");
        }
    }
}

/// `recorded` as a hook input of `session`, in the turn `turn`, Claude Code's `prompt_id`.
fn of_session(recorded: &str, session: &Standin, turn: &str) -> Value {
    let mut input: Value = serde_json::from_str(recorded).expect("a recorded input is JSON");
    input["session_id"] = json!(session.session_id());
    input["transcript_path"] = json!(session.file().to_string_lossy());
    input["prompt_id"] = json!(turn);
    input
}

/// A registered session of Ada's agent `agent`, whose hooks Claude Code runs.
pub struct Prompted<'a> {
    pub ada: &'a Ada,
    pub agent: &'static str,
    pub session: &'a Standin,
}

impl Prompted<'_> {
    /// Runs the session's hook on `input`; fails unless it prints nothing and exits with success.
    #[track_caller]
    fn tells(&self, input: &Value) {
        let event = input["hook_event_name"].as_str().unwrap_or_default();
        (self.ada.hook_of(self.agent, self.session, input, &[]))
            .assert_silent(&format!("{event} of {}", self.agent));
    }

    /// The session stops on its prompt for `call` in `turn`: its `PermissionRequest` hook.
    #[track_caller]
    pub fn requests(&self, turn: &str, call: &Call) {
        let mut input = of_session(PERMISSION_REQUEST, self.session, turn);
        input["tool_input"] = call.input();
        self.tells(&input);
    }

    /// The session's prompt in `turn` waited: its `Notification` hook of type
    /// `permission_prompt`.
    #[track_caller]
    pub fn notifies(&self, turn: &str) {
        self.tells(&of_session(NOTIFICATION, self.session, turn));
    }

    /// The session waits for the person's prompt after `turn`: its `Notification` hook of type
    /// `idle_prompt`, the recorded one with that type and its message.
    #[track_caller]
    pub fn idles(&self, turn: &str) {
        let mut input = of_session(NOTIFICATION, self.session, turn);
        input["notification_type"] = json!("idle_prompt");
        input["message"] = json!("Claude is waiting for your input");
        self.tells(&input);
    }

    /// The person allowed `call` in `turn`, which ran: the session's `PostToolUse` hook, with
    /// nothing waiting for the agent.
    #[track_caller]
    pub fn runs(&self, turn: &str, call: &Call) {
        let recorded = Event::PostToolUse.input(&self.ada.web.solo.dir(self.agent));
        let mut input = of_session(&recorded, self.session, turn);
        input["tool_input"] = call.input();
        self.tells(&input);
    }
}

/// Now, in Unix seconds.
pub fn unix_now() -> u64 {
    (SystemTime::now().duration_since(UNIX_EPOCH))
        .expect("after 1970")
        .as_secs()
}

/// The objects of [`RECORD_TYPE`] in `shown`, the output of a run with `--json`.
pub fn records(shown: &str) -> Vec<Value> {
    (shown.lines())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|object| object["type"] == RECORD_TYPE)
        .collect()
}

/// Runs the person's `skald recv`, with `args` after it, on `machine` every half second, taking
/// what `pick` finds in each run's output, until it took `count` in all; fails if no run that
/// started within [`DETECTED`] got there. Gives what it took.
#[track_caller]
fn person_finds<T>(
    machine: &Machine,
    args: &[&str],
    count: usize,
    pick: fn(&str) -> Vec<T>,
) -> Vec<T> {
    let deadline = Instant::now() + DETECTED;
    let mut found = Vec::new();
    let mut runs = Vec::new();
    let mut command = vec![RECV];
    command.extend_from_slice(args);
    loop {
        let finished = run(machine, &command);
        assert!(finished.success, "{finished:?}");
        let shown = finished.shown();
        found.extend(pick(&shown));
        runs.push(shown);
        if found.len() >= count {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "the person's skald recv showed {} prompts, not {count}, within {DETECTED:?}: {runs:#?}",
            found.len()
        );
        thread::sleep(Duration::from_millis(500));
    }
}

/// The person's `skald recv` on `machine` shows `count` lines of recorded prompts within
/// [`DETECTED`], and its next run shows none. Gives them.
#[track_caller]
pub fn person_sees(machine: &Machine, count: usize) -> Vec<String> {
    let lines = person_finds(machine, &[], count, permission_lines);
    assert_eq!(lines.len(), count, "{lines:#?}");
    person_sees_none(machine, "the run after the one that showed them");
    lines
}

/// The person's `skald recv --json` on `machine` shows `count` recorded prompts within
/// [`DETECTED`], and its next run shows none. Gives them.
#[track_caller]
pub fn person_sees_json(machine: &Machine, count: usize) -> Vec<Value> {
    let found = person_finds(machine, &[JSON_FLAG], count, records);
    assert_eq!(found.len(), count, "{found:#?}");
    let next = run(machine, &[RECV, JSON_FLAG]);
    assert!(next.success, "{next:?}");
    assert_eq!(records(&next.shown()), Vec::<Value>::new(), "{next:?}");
    found
}

/// Fails if the person's next `skald recv` on `machine` shows a recorded prompt.
#[track_caller]
pub fn person_sees_none(machine: &Machine, when: &str) {
    let next = run(machine, &[RECV]);
    assert!(next.success, "{when}: {next:?}");
    let shown = next.shown();
    assert!(permission_lines(&shown).is_empty(), "{when}: {shown}");
}

/// Fails unless `line`, the person's line of a recorded prompt, names `agent` and holds nothing of
/// `call`.
#[track_caller]
pub fn assert_line_of(line: &str, agent: &str, call: &Call) {
    assert!(holds_word(line, agent), "the line names no {agent}: {line}");
    call.assert_absent(line, "the person's line");
}

/// Fails if what `agent`'s runs show in its folder on Ada's machine holds anything of a recorded
/// prompt of `session`: its `skald recv`, with `--json` too, and the `inbox` of web's MCP session
/// if `agent` is web.
#[track_caller]
pub fn agent_sees_nothing(ada: &mut Ada, agent: &str, session: &Standin, call: &Call) {
    let dir = ada.web.solo.dir(agent);
    let mut shown = agent_recv_shown(ada.machine(), &dir, agent);
    if agent == crate::solo::WEB {
        shown.push(("inbox", ada.web.session.inbox()));
    }
    for (what, shown) in shown {
        let lower = shown.to_lowercase();
        assert!(
            !lower.contains(PERMISSION_WORD),
            "{agent}'s {what} names a prompt: {shown}"
        );
        assert!(
            !shown.contains(session.session_id()),
            "{agent}'s {what} names the session: {shown}"
        );
        call.assert_absent(&shown, &format!("{agent}'s {what}"));
    }
}
