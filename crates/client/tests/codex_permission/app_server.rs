//! The app-server's request: a Codex session on the background app-server of its config home that
//! stops on its prompt shows the person one line within a second; the request's resolution ends
//! the prompt, after either answer; a session without one is not named; a prompt both paths tell
//! shows once; the device answers no request of the app-server.

use std::thread;

use crate::codex::{Codex, Thread};
use crate::prompts::{
    PersonWaits, SETTLE, agent_sees_nothing, assert_ended, assert_line_not_of, assert_line_of,
    envelopes, person_records, person_sees_line_of, person_sees_none, prompts, record_of, sessions,
    unix_now,
};
use crate::standin::{Answer, AppServer};

/// The stand-in app-server in the config home of `sessions`' threads, hosting both; each session
/// starts, and the test waits until a client resumed each thread.
fn on_app_server(codex: &Codex) -> (AppServer, Thread, Thread) {
    let (web, api) = sessions(codex);
    let server = AppServer::new(&web.home);
    server.hosts(&web);
    server.hosts(&api);
    codex.starts(&web);
    codex.starts(&api);
    server.await_resumed(&web);
    server.await_resumed(&api);
    (server, web, api)
}

/// Fails unless the device answered no request of `server`.
#[track_caller]
fn assert_no_answers(server: &AppServer) {
    let answers = server.answers();
    assert!(answers.is_empty(), "the device answered: {answers:#?}");
}

/// web's and api's sessions run on the background app-server, and the person's `skald recv
/// --wait` waits. web's command waits on approval: within a second of the app-server's request
/// the person's run shows one line that names web and `~/work/clientA/frontend`, neither api nor
/// its folder, nothing of the command, and exits. The person's next `skald recv` shows none, and
/// the device answered no request.
#[test]
fn an_approval_request_ends_the_person_s_wait_on_one_line_within_a_second() {
    let codex = Codex::new(4_928_011_001, "ada-codex-approval-line");
    let (server, web, api) = on_app_server(&codex);
    let person = PersonWaits::start(&codex);

    let asked = server.asks(&web);

    let line = person.ends_on_line(asked);
    assert_line_of(&codex, &line, &web);
    assert_line_not_of(&codex, &line, &api);
    person_sees_none(codex.machine(), "the run after the line");
    assert_no_answers(&server);
}

/// For Ada, GitHub user `id` with `login`: web's command on the background app-server waits on
/// approval, and a few seconds later the person gives `answer`. A few seconds later the person's
/// `skald recv --json` shows one prompt, web's session's, which began when the app-server asked and
/// ended by the read; the device answered no request.
fn prompt_ends(answer: Answer, id: u64, login: &str) {
    let codex = Codex::new(id, login);
    let (server, web, _api) = on_app_server(&codex);

    let before = unix_now();
    server.asks(&web);
    let began = unix_now();
    thread::sleep(SETTLE);
    server.resolves(&web, answer);
    thread::sleep(SETTLE);
    let by = unix_now();

    let records = person_records(codex.machine());
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_ended(record_of(&codex, &records, &web, [before, began]), by);
    assert_no_answers(&server);
}

/// web's command waits on approval; a few seconds later the person answers "Yes, proceed" in the
/// terminal: the request resolved, the command completed. A few seconds later the person's `skald
/// recv --json` shows one prompt, web's session's, ended; the device answered no request.
#[test]
fn an_accepted_approval_ends_the_prompt() {
    prompt_ends(Answer::Accepted, 4_928_012_001, "ada-codex-approval-yes");
}

/// As [`an_accepted_approval_ends_the_prompt`], the person answering "No": the command declined.
#[test]
fn a_declined_approval_ends_the_prompt() {
    prompt_ends(Answer::Declined, 4_928_013_001, "ada-codex-approval-no");
}

/// web's session on the background app-server stops on its prompt, and both paths tell, as Codex
/// tells them: its `PermissionRequest` hook, then the app-server's request. A few seconds later
/// the person's `skald recv` shows one line, naming web; its next run shows none.
#[test]
fn a_prompt_both_paths_tell_shows_one_line() {
    let codex = Codex::new(4_928_014_001, "ada-codex-approval-once");
    let (server, web, _api) = on_app_server(&codex);

    prompts(&codex, &web);
    server.asks(&web);
    thread::sleep(SETTLE);

    person_sees_line_of(&codex, &web);
    person_sees_none(codex.machine(), "the run after the line");
    assert_no_answers(&server);
}

/// web's command on the background app-server waits on approval. web's `skald recv`, with
/// `--json` too, and its `inbox`, and api's `skald recv`, show nothing of it; the server stores
/// no more envelopes than before. The person's `skald recv` shows its line.
#[test]
fn an_agent_and_the_server_get_nothing_of_an_approval_request() {
    let mut codex = Codex::new(4_928_015_001, "ada-codex-approval-unseen");
    let (server, web, api) = on_app_server(&codex);
    let stored = envelopes(&codex);

    server.asks(&web);
    thread::sleep(SETTLE);

    agent_sees_nothing(&mut codex, &web, &web);
    agent_sees_nothing(&mut codex, &api, &web);
    assert_eq!(envelopes(&codex), stored, "the server stores an envelope");
    person_sees_line_of(&codex, &web);
}
