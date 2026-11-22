//! A terminal session that stops on its own permission prompt is detected within seconds: the
//! person's `skald recv` shows its line once, holding nothing of the call; a session that runs
//! without a prompt shows none; the agent's `skald recv` and `inbox` show nothing of it.
//!
//! Beside the hooks `socket_claude.rs` gives a session, `skald hook claude` is its
//! `PermissionRequest` and `Notification` hook, from the folder's `.claude/settings.local.json`,
//! which also starts it in Claude Code's default permission mode, so a call nothing allows stops on
//! the prompt. The tests never answer the prompt; they dismiss it before the session ends.

use std::fs;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::claude::claude_code;
use crate::hooks::{CLAUDE_CODE, HOOK};
use crate::kit::{RECV, received_as_agent};
use crate::prompt_lines::{PERMISSION_WORD, permission_lines};
use crate::senders::holds_word;
use crate::socket_claude::{Session, Surface};
use crate::socket_model::{Model, Reply, last_user_text};
use crate::socket_sessions::{QUIET, after_a_tool};
use crate::solo::{Solo, WEB};
use crate::world::{AGENT, run};

/// The prompt of the turn whose call its settings allow.
const ALLOWED: &str = "e2e permission test: run an allowed command";

/// The prompt of the turn whose call nothing allows, and the call.
const PROMPTED: &str = "e2e permission test: run a command nothing allows";
const COMMAND: &str = "touch prompted-e2e.txt";

/// How soon the person's `skald recv` shows the prompt after the stub's call.
const DETECTED: Duration = Duration::from_secs(5);

/// How long after the stub's call the test keeps reading: past the `Notification` hook, which runs
/// about 6 s after the prompt shows.
const WATCHED: Duration = Duration::from_secs(12);

/// Runs the person's `skald recv` on Ada's machine; gives its output.
#[track_caller]
fn person_reads(solo: &Solo) -> String {
    let finished = run(solo.machine(), &[RECV]);
    assert!(finished.success, "{finished:?}");
    finished.shown()
}

/// Makes `skald --agent web hook claude` web's `PermissionRequest` and `Notification` hook, and
/// Claude Code's default permission mode its mode, in web's `.claude/settings.local.json`.
fn prompt_hooks(solo: &Solo) {
    let dir = solo.dir(WEB);
    let skald = solo
        .machine()
        .command_in(&dir, &[AGENT, WEB, HOOK, CLAUDE_CODE]);
    let program = skald.get_program().to_string_lossy().into_owned();
    let hook = json!([{ "hooks": [{
        "type": "command", "command": program, "args": [AGENT, WEB, HOOK, CLAUDE_CODE],
    }] }]);
    let settings = json!({
        "permissions": { "defaultMode": "default" },
        "hooks": { "PermissionRequest": hook, "Notification": hook },
    });
    let folder = dir.join(".claude");
    fs::create_dir_all(&folder).expect("make the folder's .claude");
    let text = serde_json::to_string_pretty(&settings).expect("JSON encodes");
    fs::write(folder.join("settings.local.json"), text).expect("write the local settings");
}

/// A terminal session of web in `~/work/clientA/web` takes a prompt; its stub calls `true`, which
/// its settings allow, and the turn ends: the person's `skald recv` shows no prompt for [`QUIET`].
/// It takes another prompt; its stub calls `touch`, which nothing allows, and the session stops on
/// the prompt. A run of the person's `skald recv` that starts within [`DETECTED`] of the call shows
/// one line that names web and holds nothing of the call; the runs until [`WATCHED`] after it show
/// no other. web's `skald recv` and `inbox` show nothing of it.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_session_waiting_on_its_permission_prompt_is_detected_within_seconds() {
    let Some(claude) = claude_code() else { return };
    let solo = Solo::new(4_919_707_001, "ada-waiting-e2e");
    prompt_hooks(&solo);
    let called: Arc<Mutex<Option<Instant>>> = Arc::default();
    let at = Arc::clone(&called);
    let model = Model::start(move |body| {
        let said = last_user_text(body);
        if after_a_tool(body) {
            Reply::Done
        } else if said.contains(PROMPTED) {
            let mut at = at.lock().unwrap_or_else(PoisonError::into_inner);
            at.get_or_insert_with(Instant::now);
            Reply::Bash(COMMAND)
        } else if said.contains(ALLOWED) {
            Reply::Bash("true")
        } else {
            Reply::Done
        }
    });
    let mut session = Session::start(
        &claude,
        solo.machine(),
        &solo.dir(WEB),
        WEB,
        &model,
        Surface::Terminal,
    );

    session.prompt(ALLOWED);
    session.wait_turn_end(&model, ALLOWED, 1);
    let until = Instant::now() + QUIET;
    while Instant::now() < until {
        let shown = person_reads(&solo);
        let lines = permission_lines(&shown);
        assert!(lines.is_empty(), "a prompt without one: {shown}");
        thread::sleep(Duration::from_millis(500));
    }

    session.prompt(PROMPTED);
    session.wait_turn_holding(&model, PROMPTED);
    let called = loop {
        if let Some(at) = *called.lock().unwrap_or_else(PoisonError::into_inner) {
            break at;
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut seen: Vec<(Duration, String)> = Vec::new();
    while called.elapsed() < WATCHED {
        let started = called.elapsed();
        let shown = person_reads(&solo);
        seen.extend(
            permission_lines(&shown)
                .into_iter()
                .map(|line| (started, line)),
        );
        thread::sleep(Duration::from_millis(500));
    }
    let [(started, line)] = seen.as_slice() else {
        panic!(
            "not one line of the prompt within {WATCHED:?}: {seen:#?}; the session printed: {}",
            session.output()
        );
    };
    assert!(
        *started <= DETECTED,
        "shown by a run {started:?} after the call"
    );
    assert!(holds_word(line, WEB), "the line names no {WEB}: {line}");
    for part in [COMMAND, "prompted-e2e", "touch", "Bash"] {
        assert!(!line.contains(part), "the line holds {part:?}: {line}");
    }

    let agent = received_as_agent(solo.machine(), &solo.dir(WEB), WEB).shown();
    let inbox = solo.session(WEB).inbox();
    for (what, shown) in [("skald recv", agent), ("inbox", inbox)] {
        assert!(
            !shown.to_lowercase().contains(PERMISSION_WORD) && !shown.contains("prompted-e2e"),
            "web's {what} shows the prompt: {shown}"
        );
    }
    model.assert_no_foreign_credential();

    // A session left on its prompt does not exit when asked, and Windows keeps the machine's
    // directory while its processes run.
    session.dismiss_prompt();
    session.end();
}
