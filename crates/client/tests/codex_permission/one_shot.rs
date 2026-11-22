//! The person's one-shot `skald recv --wait`, with no helper: it ends on a prompt that a
//! `PermissionRequest` hook records.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::codex::{Codex, Event, hook_command};
use crate::helped::ONE_SHOT_SETTING;
use crate::hooks::run_hook;
use crate::kit::{RECV, received};
use crate::prompts::{
    IDLE_WATCH, WAIT_FLAG, WITHIN, assert_line_of, permission_lines, person_sees_none, sessions,
};

/// The settings of a hook that starts no helper.
const NO_HELPER: [(&str, &str); 1] = [(ONE_SHOT_SETTING, "1")];

/// web's session starts, its hooks in one-shot mode, so no helper runs, and the person's one-shot
/// `skald recv --wait` waits. web's session stops on its prompt: within a second of its
/// `PermissionRequest` hook the person's run shows one line naming web and exits; the next run
/// shows none.
#[test]
fn a_one_shot_wait_ends_on_a_permission_request_within_a_second() {
    let codex = Codex::new(4_928_008_001, "ada-codex-prompt-one-shot");
    let (web, _api) = sessions(&codex);
    (codex.hook_with(&web, Event::SessionStart, &NO_HELPER)).assert_silent("SessionStart");
    received(codex.machine());
    let mut waits = codex.machine().start(&[RECV, WAIT_FLAG]);
    thread::sleep(IDLE_WATCH);
    waits.assert_running();

    let mut input: Value = serde_json::from_str(include_str!("recorded/permission-request.json"))
        .expect("a recorded input is JSON");
    input["session_id"] = json!(web.id);
    input["cwd"] = json!(web.dir.to_string_lossy());
    let hooked = run_hook(
        &mut hook_command(&codex.helped, &web, &NO_HELPER),
        &input.to_string(),
    );
    let told = Instant::now();
    hooked.assert_silent("PermissionRequest");

    while permission_lines(&waits.shown()).is_empty() {
        assert!(
            told.elapsed() <= WITHIN,
            "no line within {WITHIN:?}: {}",
            waits.shown()
        );
        assert!(
            !waits.exited(),
            "it exited without a line: {}",
            waits.shown()
        );
        thread::sleep(Duration::from_millis(10));
    }
    let finished = waits.finish();
    assert!(finished.success, "{finished:?}");
    let lines = permission_lines(&finished.shown());
    let [line] = lines.as_slice() else {
        panic!("not one line: {finished:?}");
    };
    assert_line_of(&codex, line, &web);
    person_sees_none(codex.machine(), "the run after the line");
}
