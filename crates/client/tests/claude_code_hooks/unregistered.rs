//! A session that never registered, such as one with no cross-session socket, has its permission
//! prompt recorded too: the person's `skald recv` names its agent and the agent's space.

use serde_json::{Value, json};

use crate::hooks::{Web, hook_command, run_hook};
use crate::kit::RECV;
use crate::lines::JSON_FLAG;
use crate::senders::holds_word;
use crate::solo::WEB;

/// The hook inputs of a prompt, recorded from Claude Code 2.1.328.
const PERMISSION_REQUEST: &str =
    include_str!("../claude_code_permission/recorded/permission-request.json");
const NOTIFICATION: &str = include_str!("../claude_code_permission/recorded/notification.json");

/// The space of web's folder.
const SPACE: &str = "clienta";

/// Runs web's hook handler in its folder on `recorded` as the input of `session`, which ran no
/// `SessionStart`; fails unless it prints nothing.
#[track_caller]
fn tells(web: &Web, recorded: &str, session: &str) {
    let dir = web.dir();
    let mut input: Value = serde_json::from_str(recorded).expect("a recorded input is JSON");
    input["cwd"] = json!(dir.to_string_lossy());
    input["session_id"] = json!(session);
    input["transcript_path"] = json!(dir.join(format!("{session}.jsonl")).to_string_lossy());
    let event = input["hook_event_name"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    run_hook(
        &mut hook_command(web.machine(), &dir, Some(WEB)),
        &input.to_string(),
    )
    .assert_silent(&event);
}

/// A session of web that ran no `SessionStart` stops on its prompt: `PermissionRequest`. The
/// person's `skald recv` shows one line that names web and `clienta`. Another such session's prompt
/// is told by `Notification` alone: the person's `skald recv --json` shows one record with web,
/// `clienta` and that session.
#[test]
fn a_prompt_of_a_session_that_never_registered_is_recorded() {
    let web = Web::new(4_913_022_001, "ada-hooks-unregistered");

    tells(
        &web,
        PERMISSION_REQUEST,
        "b1b1b1b1-0000-4000-8000-000000000001",
    );
    let shown = web.solo.person_reads();
    let lines: Vec<&str> = (shown.lines())
        .filter(|line| line.to_lowercase().contains("permission"))
        .collect();
    let [line] = lines.as_slice() else {
        panic!("not one line of the prompt: {shown}");
    };
    assert!(holds_word(line, WEB), "the line names no {WEB}: {line}");
    assert!(holds_word(line, SPACE), "the line names no {SPACE}: {line}");

    let session = "b1b1b1b1-0000-4000-8000-000000000002";
    tells(&web, NOTIFICATION, session);
    let read = web.solo.person_runs(&[RECV, JSON_FLAG]);
    assert!(read.success, "{read:?}");
    let records: Vec<Value> = (read.shown().lines())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|object| object["type"] == "permission_prompt")
        .collect();
    let [record] = records.as_slice() else {
        panic!("not one record of the prompt: {read:?}");
    };
    assert_eq!(
        (&record["agent"], &record["space"], &record["session"]),
        (&json!(WEB), &json!(SPACE), &json!(session)),
        "{record}"
    );
}
