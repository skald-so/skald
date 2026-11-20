//! A session that refuses cross-session messages gets them by the hooks. A helper killed after the
//! write neither loses the message nor writes it twice, and `read` of it goes once. The client keeps
//! nothing of the conversation: a secret planted in it is in no file of the local state.

use std::sync::{Arc, Condvar, Mutex};

use serde_json::json;

use crate::claude::claude_code;
use crate::helped::Helped;
use crate::receipts::shows_read_once;
use crate::socket_claude::{Options, SOCKET, Session, Surface};
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::socket_sessions::{QUIET, START, after_a_tool, assert_own_in, nudged_unless_socket};
use crate::solo::{API, Solo, WEB};

/// What api says to web in each test; none is anywhere else.
const REFUSED_SAYS: &str = "e2e socket test: api asks web, which refuses messages";
const KILLED_SAYS: &str = "e2e socket test: api asks web as its helper dies";

/// The person's prompt to the session that refused the message.
const AGAIN: &str = "e2e socket test: carry on";

/// What the conversation holds that no message does, and the command that puts it there.
const PLANTED: &str = "planted-secret-0d9e4c7a21b6f358";
const ECHO_PLANTED: &str = "echo planted-secret-0d9e4c7a21b6f358";

/// Starts a session of web as the apps run it, with `options`, gives it [`START`], and waits until
/// its turn ended.
fn idle_web(solo: &Solo, model: &Model, options: Options) -> Option<Session> {
    let claude = claude_code()?;
    let place = (solo.machine(), solo.dir(WEB));
    let mut session = Session::start_with(
        &claude,
        (place.0, &place.1),
        WEB,
        model,
        Surface::App,
        options,
    );
    session.prompt(START);
    session.wait_turn_end(model, START, 1);
    Some(session)
}

/// A session of web that refuses cross-session messages, `crossSessionInbound` `refuse`, runs as the
/// apps run it; it takes the prompt, and its turn ends. api sends web a message; then the person
/// gives another prompt, on which the stub runs `sleep 1` until a turn holds the message: that turn
/// holds it as its line from api at `own`, after a tool's result, and the person's `skald recv`
/// shows `read` of it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_session_that_refuses_cross_session_messages_gets_them_by_the_hooks() {
    let solo = Solo::new(4_919_456_001, "ada-socket-e2e-refusing");
    let model = Model::start(|body| {
        let said = last_user_text(body);
        if conversation(body).contains(REFUSED_SAYS) {
            Reply::Done
        } else if said.contains(AGAIN) || after_a_tool(body) {
            Reply::Bash("sleep 1")
        } else {
            Reply::Done
        }
    });
    let mut settings = serde_json::Map::new();
    settings.insert("crossSessionInbound".to_owned(), json!("refuse"));
    let options = Options {
        settings,
        ..Options::default()
    };
    let Some(mut session) = idle_web(&solo, &model, options) else {
        return;
    };

    let msg_id = solo.api_sends_web(REFUSED_SAYS);
    session.prompt(AGAIN);

    let turn = model.wait_turn_holding(REFUSED_SAYS, "api's message after the refusal");
    assert!(after_a_tool(&turn), "not after a tool: {turn}");
    assert_own_in(&turn, REFUSED_SAYS, msg_id, &solo.short(API));
    shows_read_once(solo.machine(), msg_id);
    model.assert_no_foreign_credential();
}

/// A session of web runs as the apps run it, its hooks run by a copy of `skald` whose processes the
/// test finds, its `skald mcp` in one-shot mode, so only the hooks start a helper. On the prompt,
/// the stub runs `echo` of [`PLANTED`], which the conversation then holds. api sends web a message;
/// once a turn holds it, before the stub answers, every helper is killed where the client writes
/// into the socket ([`SOCKET`]), and then the stub replies. The person's `skald recv` shows `read` of the message once; after [`QUIET`] more, the
/// session's last turn holds it once; no file of the state directory holds [`PLANTED`].
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_helper_killed_after_the_write_gives_read_once_and_keeps_nothing_of_the_conversation() {
    let solo = Solo::new(4_919_457_001, "ada-socket-e2e-helper-killed");
    let helped = Helped::new(solo.machine());
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let waiting = Arc::clone(&gate);
    let model = Model::start(move |body| {
        let said = last_user_text(body);
        if said.contains(KILLED_SAYS) {
            let (open, opened) = &*waiting;
            let mut open = open.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            while !*open {
                open = opened
                    .wait(open)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
        if !after_a_tool(body) && said.contains(START) {
            Reply::Bash(ECHO_PLANTED)
        } else {
            Reply::Done
        }
    });
    let options = Options {
        hooks: Some(helped.exe().to_owned()),
        ..Options::default()
    };
    let Some(mut session) = idle_web(&solo, &model, options) else {
        return;
    };
    let echoed = conversation(model.turns().last().expect("a turn"));
    assert!(
        echoed.contains(PLANTED),
        "the conversation holds no {PLANTED}"
    );

    let msg_id = solo.api_sends_web(KILLED_SAYS);
    nudged_unless_socket(&mut session);
    model.wait_turn_holding(KILLED_SAYS, "api's message");
    // Where the client writes into no socket, no write comes before the hooks give the message.
    if SOCKET {
        let helpers = helped.helpers();
        assert!(!helpers.is_empty(), "no helper ran");
        for helper in helpers {
            helped.kill(helper);
        }
    }
    let (open, opened) = &*gate;
    *open.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    opened.notify_all();

    shows_read_once(solo.machine(), msg_id);
    std::thread::sleep(QUIET);
    let last = conversation(model.turns().last().expect("a turn"));
    assert_eq!(last.matches(KILLED_SAYS).count(), 1, "not once: {last}");
    let state = solo.machine().state_dir();
    let kept: Vec<_> = (solo.machine().files_holding(PLANTED).into_iter())
        .filter(|file| file.starts_with(&state))
        .collect();
    assert!(kept.is_empty(), "{PLANTED} kept in {kept:?}");
    model.assert_no_foreign_credential();
}
