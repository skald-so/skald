//! The `PermissionRequest` hook: a Codex session that stops on its prompt shows the person one
//! line within a second; the call's `PostToolUse`, the next turn and the exit of its Codex process
//! end the prompt; a session without one is not named; nothing reaches an agent or the server.

use std::thread;

use crate::codex::{Codex, CodexProcess, Event, QUIET, Thread};
use crate::prompts::{
    EXIT_NOTICED, PersonWaits, agent_sees_nothing, allowed, assert_ended, assert_line_not_of,
    assert_line_of, assert_waits, envelopes, next_turn, person_records, person_sees_line_of,
    person_sees_none, prompts, prompts_by, record_of, sessions, unix_now, web_and_api, whole_turn,
};

/// web's and api's sessions start, and the person's `skald recv --wait` waits. web's session
/// stops on its prompt: its `PermissionRequest` hook prints nothing, and within a second the
/// person's run shows one line that names web and `~/work/clientA/frontend`, holding nothing of
/// the command, and exits. The person's next `skald recv` shows none.
#[test]
fn a_permission_request_ends_the_person_s_wait_on_one_line_within_a_second() {
    let codex = Codex::new(4_928_001_001, "ada-codex-prompt-line");
    let (web, api) = web_and_api(&codex);
    let person = PersonWaits::start(&codex);

    let told = prompts(&codex, &web);

    let line = person.ends_on_line(told);
    assert_line_of(&codex, &line, &web);
    assert_line_not_of(&codex, &line, &api);
    person_sees_none(codex.machine(), "the run after the line");
}

/// web's session stops on its prompt. The person's `skald recv --json` shows one prompt: web's
/// session, its agent, when the hook ran, not ended, nothing of the command.
#[test]
fn the_person_s_json_names_the_session_and_when_it_began_to_wait() {
    let codex = Codex::new(4_928_002_001, "ada-codex-prompt-json");
    let (web, _api) = web_and_api(&codex);

    let before = unix_now();
    prompts(&codex, &web);
    let after = unix_now();

    let records = person_records(codex.machine());
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_waits(record_of(&codex, &records, &web, [before, after]));
}

/// For Ada, GitHub user `id` with `login`: web's session stops on its prompt, then `ends` runs.
/// The person's `skald recv --json` shows the prompt, ended by then.
fn ended_by(ends: fn(&Codex, &Thread), id: u64, login: &str) {
    let codex = Codex::new(id, login);
    let (web, _api) = web_and_api(&codex);

    let before = unix_now();
    prompts(&codex, &web);
    let began = unix_now();
    ends(&codex, &web);
    let by = unix_now();

    let records = person_records(codex.machine());
    assert_ended(record_of(&codex, &records, &web, [before, began]), by);
}

/// web's session stops on its prompt, and the person allows the command: its `PostToolUse` prints
/// nothing. The person's `skald recv --json` shows the prompt, ended by then.
#[test]
fn the_call_s_post_tool_use_ends_the_prompt() {
    ended_by(allowed, 4_928_003_001, "ada-codex-prompt-allowed");
}

/// web's session stops on its prompt; the person declines the command, which runs no hook, and
/// types the next prompt: the next turn's `UserPromptSubmit` prints nothing. The person's `skald
/// recv --json` shows the prompt, ended by then.
#[test]
fn the_next_turn_ends_a_prompt_the_person_declined() {
    ended_by(next_turn, 4_928_004_001, "ada-codex-prompt-declined");
}

/// api's session starts, and web's, whose hooks a Codex process runs, starts and stops on its
/// prompt; then the process exits. A few seconds later the person's `skald recv --json` shows the
/// prompt, ended by then.
#[test]
fn a_prompt_ends_when_its_codex_process_exits() {
    let codex = Codex::new(4_928_005_001, "ada-codex-prompt-exit");
    let (web, api) = sessions(&codex);
    codex.starts(&api);
    let mut process = CodexProcess::start();
    codex
        .hook_by(&mut process, &web, Event::SessionStart)
        .assert_silent("SessionStart");

    let before = unix_now();
    prompts_by(&codex, &mut process, &web);
    let began = unix_now();
    process.end();
    thread::sleep(EXIT_NOTICED);
    let by = unix_now();

    let records = person_records(codex.machine());
    assert_ended(record_of(&codex, &records, &web, [before, began]), by);
}

/// web's and api's sessions start; api's runs whole turns without a prompt, each hook printing
/// nothing, before and after web's session stops on its prompt. A few seconds later the person's
/// `skald recv --json` shows one prompt, web's session's, and none of api's.
#[test]
fn a_second_session_without_a_prompt_is_not_named() {
    let codex = Codex::new(4_928_006_001, "ada-codex-prompt-other");
    let (web, api) = web_and_api(&codex);

    whole_turn(&codex, &api);
    let before = unix_now();
    prompts(&codex, &web);
    let began = unix_now();
    whole_turn(&codex, &api);
    thread::sleep(QUIET);

    let records = person_records(codex.machine());
    assert_eq!(records.len(), 1, "{records:#?}");
    record_of(&codex, &records, &web, [before, began]);
}

/// web's session stops on its prompt. web's `skald recv`, with `--json` too, and its `inbox`, and
/// api's `skald recv`, show nothing of it; web's `PostToolUse` and `Stop` print nothing; the
/// server stores no more envelopes than before. The person's `skald recv` shows its line.
#[test]
fn an_agent_and_the_server_get_nothing_of_a_prompt() {
    let mut codex = Codex::new(4_928_007_001, "ada-codex-prompt-unseen");
    let (web, api) = web_and_api(&codex);
    let stored = envelopes(&codex);

    prompts(&codex, &web);
    thread::sleep(QUIET);

    agent_sees_nothing(&mut codex, &web, &web);
    agent_sees_nothing(&mut codex, &api, &web);
    for event in [Event::PostToolUse, Event::Stop] {
        codex.silent(&web, event, "after the prompt");
    }
    assert_eq!(envelopes(&codex), stored, "the server stores an envelope");
    person_sees_line_of(&codex, &web);
}
