//! What the tests share: Ada's machine with web's MCP session, as the tests of the hook handler
//! have it, the copy of `skald` that Claude Code's hooks run without the one-shot setting, so the
//! session's `SessionStart` starts the helper, as in the tests of the helper, and the hook inputs
//! of a session the stand-in plays.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::cloud::WAIT;
use crate::helped::Helped;
use crate::hooks::{CLAUDE_CODE, Event, HOOK, Hooked, Web, run_hook};
use crate::machine::Machine;
use crate::receipts::read_lines;
use crate::standin::{SOCKET_VAR, Standin, TOKEN_VAR};
use crate::world::AGENT;

/// Hook inputs recorded from Claude Code 2.1.329, with the paths of a temporary directory.
const SESSION_START: &str = include_str!("recorded/session-start.json");
const SESSION_END: &str = include_str!("recorded/session-end.json");

/// How long a test watches for what must not happen.
pub const QUIET: Duration = Duration::from_secs(5);

/// Ada's machine, web's MCP session, and the copy of `skald` the hooks run.
pub struct Ada {
    /// Kills the helpers before the machine's directory goes.
    pub helped: Helped,
    pub web: Web,
}

impl Ada {
    /// Ada, GitHub user `id` with `login`, alone in `clienta`.
    pub fn new(id: u64, login: &str) -> Self {
        let web = Web::new(id, login);
        let helped = Helped::new(web.machine());
        Ada { helped, web }
    }

    pub fn machine(&self) -> &Machine {
        self.web.machine()
    }

    /// Claude Code runs `skald --agent <agent> hook claude` in the agent's folder on `input`, with
    /// `settings`, as a hook of `session`'s: with its socket and token but on `SessionEnd`.
    pub fn hook_of(
        &self,
        agent: &str,
        session: &Standin,
        input: &Value,
        settings: &[(&str, &str)],
    ) -> Hooked {
        let dir = self.web.solo.dir(agent);
        let socket = session.socket().to_string_lossy().into_owned();
        let mut given: Vec<(&str, &str)> = settings.to_vec();
        if input["hook_event_name"] != "SessionEnd" {
            given.extend([(SOCKET_VAR, socket.as_str()), (TOKEN_VAR, session.token())]);
        }
        let mut command =
            (self.helped).command_in(&dir, &given, &[AGENT, agent, HOOK, CLAUDE_CODE]);
        let mut input = input.clone();
        input["cwd"] = json!(dir.to_string_lossy());
        run_hook(&mut command, &input.to_string())
    }

    /// `session`'s `SessionStart` for `agent`, with `settings`; fails unless it prints nothing.
    #[track_caller]
    pub fn starts_with(&self, agent: &str, session: &Standin, settings: &[(&str, &str)]) {
        let hooked = self.hook_of(
            agent,
            session,
            &of_session(SESSION_START, session),
            settings,
        );
        hooked.assert_silent("SessionStart");
    }

    /// As [`Ada::starts_with`], without settings.
    #[track_caller]
    pub fn starts(&self, agent: &str, session: &Standin) {
        self.starts_with(agent, session, &[]);
    }

    /// `session`'s `SessionEnd` for `agent`; fails unless it prints nothing.
    #[track_caller]
    pub fn ends(&self, agent: &str, session: &Standin) {
        let hooked = self.hook_of(agent, session, &of_session(SESSION_END, session), &[]);
        hooked.assert_silent("SessionEnd");
    }

    /// `session`'s hook of `event` for `agent`.
    pub fn hook(&self, agent: &str, session: &Standin, event: Event) -> Hooked {
        let dir = self.web.solo.dir(agent);
        let recorded: Value = serde_json::from_str(&event.input(&dir)).expect("an input is JSON");
        let input = with_session(recorded, session);
        self.hook_of(agent, session, &input, &[])
    }

    /// Runs `session`'s `PostToolUse` for `agent` every half second until it gives the session
    /// something, for up to [`WAIT`]; fails if none does. Gives its text.
    #[track_caller]
    pub fn hook_gives(&self, agent: &str, session: &Standin) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            let hooked = self.hook(agent, session, Event::PostToolUse);
            if !hooked.stdout.is_empty() {
                return hooked.context(Event::PostToolUse);
            }
            hooked.assert_silent("PostToolUse with nothing to give");
            assert!(
                Instant::now() < deadline,
                "PostToolUse gave nothing within {WAIT:?}"
            );
            thread::sleep(Duration::from_millis(500));
        }
    }
}

/// The recorded input `recorded` as `session`'s.
fn of_session(recorded: &str, session: &Standin) -> Value {
    with_session(
        serde_json::from_str(recorded).expect("a recorded input is JSON"),
        session,
    )
}

fn with_session(mut input: Value, session: &Standin) -> Value {
    input["session_id"] = json!(session.session_id());
    input["transcript_path"] = json!(session.file().to_string_lossy());
    input
}

/// Runs the person's `skald recv` on `machine` every half second for `quiet`; fails if a run
/// shows `read` of `msg_id`.
#[track_caller]
pub fn no_read_for(machine: &Machine, msg_id: MessageId, quiet: Duration, when: &str) {
    let until = Instant::now() + quiet;
    loop {
        let shown = crate::kit::received(machine).shown();
        assert_eq!(read_lines(&shown, msg_id), 0, "{when}: {shown}");
        if Instant::now() > until {
            return;
        }
        thread::sleep(Duration::from_millis(500));
    }
}

/// Waits for `quiet`; fails if `session`'s socket got more than `count` lines holding `text`
/// meanwhile, or ever.
#[track_caller]
pub fn lines_stay(session: &Standin, text: &str, count: usize, quiet: Duration) {
    thread::sleep(quiet);
    let holding = session.lines_holding(text);
    assert_eq!(holding.len(), count, "lines holding {text:?}: {holding:#?}");
}
