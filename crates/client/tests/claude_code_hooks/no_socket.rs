//! A session for which Claude Code gives no cross-session socket, or whose socket takes no write,
//! as on Windows, where Claude Code's socket is a named pipe, gets its messages by the hooks: its
//! `SessionStart` and `SessionEnd` print nothing, and `PostToolUse` gives what waits.

use std::fs;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::helped::Helped;
use crate::hooks::{CLAUDE_CODE, Event, HOOK, Hooked, Web, assert_own_line, run_hook};
use crate::receipts::shows_read_once;
use crate::solo::WEB;
use crate::world::AGENT;

/// The `SessionStart` input recorded from Claude Code 2.1.329.
const SESSION_START: &str = include_str!("../claude_code_socket/recorded/session-start.json");

/// The settings that give a hook the session's socket and token.
const SOCKET_SETTING: &str = "CLAUDE_CODE_MESSAGING_SOCKET";
const TOKEN_SETTING: &str = "CLAUDE_CODE_MESSAGING_TOKEN";

/// What api says to web in each session; none is anywhere else.
const WITHOUT_SOCKET: &str = "no socket test: api asks web in a session with no socket";
const DEAD_SOCKET: &str = "no socket test: api asks web in a session whose socket takes nothing";

/// Runs web's hook handler in its folder through `helped`, as Claude Code runs it, on `input`,
/// with `settings`.
fn hook(web: &Web, helped: &Helped, input: &Value, settings: &[(&str, &str)]) -> Hooked {
    let dir = web.dir();
    let mut command = helped.command_in(&dir, settings, &[AGENT, WEB, HOOK, CLAUDE_CODE]);
    let mut input = input.clone();
    input["cwd"] = json!(dir.to_string_lossy());
    run_hook(&mut command, &input.to_string())
}

/// A session of web starts with no socket, and then another whose socket is a file that takes no
/// write; the hooks run as Claude Code runs them, through a helper. In each, `SessionStart` prints
/// nothing; api sends web a message; `PostToolUse` gives it as its line from api at `own`, and the
/// person's `skald recv` shows `read` of it once; `SessionEnd` prints nothing.
#[test]
fn a_session_without_a_socket_that_takes_a_write_gets_its_messages_by_the_hooks() {
    let web = Web::new(4_919_465_001, "ada-hooks-no-socket");
    let helped = Helped::new(web.machine());
    let dead = web.machine().root().join("tmp").join("dead.sock");
    fs::create_dir_all(dead.parent().expect("a parent")).expect("make the directory");
    fs::write(&dead, "").expect("write the file that stands for the socket");
    let dead = dead.to_string_lossy().into_owned();
    let token = "0123456789abcdef0123456789abcdef";
    let cases = [
        (
            "a0a0a0a0-0000-4000-8000-000000000001",
            WITHOUT_SOCKET,
            Vec::new(),
        ),
        (
            "a0a0a0a0-0000-4000-8000-000000000002",
            DEAD_SOCKET,
            vec![(SOCKET_SETTING, dead.as_str()), (TOKEN_SETTING, token)],
        ),
    ];
    for (session, says, settings) in cases {
        let mut start: Value = serde_json::from_str(SESSION_START).expect("JSON");
        start["session_id"] = json!(session);
        let file = web
            .machine()
            .root()
            .join("tmp")
            .join(format!("{session}.jsonl"));
        start["transcript_path"] = json!(file.to_string_lossy());
        hook(&web, &helped, &start, &settings).assert_silent("SessionStart");

        let msg_id = web.own(says);
        // The helper takes the message in and tries the session's socket first, if any.
        thread::sleep(Duration::from_secs(3));

        let mut tool: Value = serde_json::from_str(&Event::PostToolUse.input(&web.dir()))
            .expect("a recorded input is JSON");
        tool["session_id"] = json!(session);
        let context = hook(&web, &helped, &tool, &settings).context(Event::PostToolUse);
        assert_own_line(&context, says, msg_id, &web.api());
        shows_read_once(web.machine(), msg_id);
        let mut end = start.clone();
        end["hook_event_name"] = json!("SessionEnd");
        end["reason"] = json!("other");
        hook(&web, &helped, &end, &[]).assert_silent("SessionEnd");
    }
}
