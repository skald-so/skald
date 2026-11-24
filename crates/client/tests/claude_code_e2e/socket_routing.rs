//! Each message goes to a running session of its own agent: of two agents' sessions, each gets only
//! its own agent's messages; of two sessions of one agent, the one registered last gets each.

use std::thread;

use crate::hooks::api_sends_web;
use crate::kit::received;
use crate::receipts::{READ, read_lines, shows_read};
use crate::socket_claude::Surface;
use crate::socket_model::{Model, Reply, conversation};
use crate::socket_sessions::{QUIET, assert_own_in, idle_session, nudged_unless_socket};
use crate::solo::{API, Solo, WEB};

/// What api and web say to each other; neither is anywhere else.
const TO_WEB: &str = "e2e socket test: api asks web for the schema";
const TO_API: &str = "e2e socket test: web asks api for the routes";

/// Fails if a turn `model` got holds `text`.
#[track_caller]
fn never_got(model: &Model, text: &str, session: &str) {
    for turn in model.turns() {
        let held = conversation(&turn);
        assert!(!held.contains(text), "{session} got {text:?}: {held}");
    }
}

/// Two sessions of web start as the apps run them, the older one first, then one of api; each takes
/// the prompt, and its turn ends. api sends web a message, and web's MCP session sends api one. A
/// turn of the newer session of web holds api's message as its line from api at `own`, and one of
/// api's session holds web's as its line from web; the person's `skald recv` shows `read` of each
/// once. After [`QUIET`] more, no turn of the older session of web holds either, and neither of the
/// others holds the message for the other agent.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn each_message_reaches_one_session_of_its_own_agent() {
    let solo = Solo::new(4_919_455_001, "ada-socket-e2e-routing");
    let models = [(); 3].map(|()| Model::start(|_| Reply::Done));
    let [older_model, newer_model, api_model] = &models;
    let Some(older) = idle_session(&solo, WEB, older_model, Surface::App) else {
        return;
    };
    let Some(mut newer) = idle_session(&solo, WEB, newer_model, Surface::App) else {
        return;
    };
    let Some(mut api) = idle_session(&solo, API, api_model, Surface::App) else {
        return;
    };

    let to_web = api_sends_web(&solo, TO_WEB);
    let to_api = solo.sends(&mut solo.session(WEB), &solo.short(API), TO_API);
    nudged_unless_socket(&mut newer);
    nudged_unless_socket(&mut api);

    // A failed wait shows each session's hook events: whether its hooks ran, and how they exited.
    let hooks = || {
        let sessions = [
            ("web's older", &older),
            ("web's newer", &newer),
            ("api's", &api),
        ];
        (sessions.iter())
            .map(|(name, session)| format!("{name} session's hooks:\n{}", session.hook_events()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let what = "api's message in web's newer session";
    let turn = newer_model.wait_turn_showing(TO_WEB, what, &hooks);
    assert_own_in(&turn, TO_WEB, to_web, &solo.short(API));
    let turn = api_model.wait_turn_showing(TO_API, "web's message in api's session", &hooks);
    assert_own_in(&turn, TO_API, to_api, &solo.short(WEB));
    // The person's `skald recv` shows each `read` once, both maybe in one run.
    let mut shown = shows_read(solo.machine(), to_web);
    if read_lines(&shown, to_api) == 0 {
        shown += &shows_read(solo.machine(), to_api);
    }
    let next = received(solo.machine()).shown();
    for msg_id in [to_web, to_api] {
        assert_eq!(read_lines(&shown, msg_id), 1, "{READ} not once: {shown}");
        assert_eq!(read_lines(&next, msg_id), 0, "{READ} again: {next}");
    }
    thread::sleep(QUIET);
    never_got(older_model, TO_WEB, "web's older session");
    never_got(older_model, TO_API, "web's older session");
    never_got(newer_model, TO_API, "web's newer session");
    never_got(api_model, TO_WEB, "api's session");
    for model in &models {
        model.assert_no_foreign_credential();
    }
}
