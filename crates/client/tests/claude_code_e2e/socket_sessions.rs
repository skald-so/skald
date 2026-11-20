//! A running Claude Code session whose `SessionStart` hook registered it gets its agent's messages
//! over its cross-session socket: idle, without a prompt, and busy, after the running tool; in a
//! terminal and as the desktop app and VS Code run it. An `own` message reaches the model as its
//! line, a `peer` one only as the signal, whose `inbox` call then gives its text. `read` goes once
//! the model's own entry follows the message in the session file, once, and not after an API error.
//! Where the client writes into no socket ([`SOCKET`]), the hooks give the messages: an idle
//! session gets them once the person gives it a prompt.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::MessageId;

use crate::claude::claude_code;
use crate::hooks::{Bob, INBOX_WORD, assert_own_line, assert_signal};
use crate::lines::agent_line_of;
use crate::machine::Machine;
use crate::receipts::{read_lines, shows_read_once};
use crate::senders::{DATA, OWN, PEER, holds_word};
use crate::sessions::{assert_from, line_in};
use crate::socket_claude::{SOCKET, Session, Surface};
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::solo::{API, Solo, WEB};
use crate::talk::{ADA_HOST, BOB_HOST};

/// The prompt that opens each session; then it is idle.
pub const START: &str = "e2e socket test: wait for messages";

/// The prompt that the person gives an idle session where the client writes into no socket.
const NUDGE: &str = "e2e socket test: anything new?";

/// How long a test watches for what must not happen.
pub const QUIET: Duration = Duration::from_secs(10);

/// Where the client writes into no socket ([`SOCKET`]), the person gives `session` a prompt, and
/// its `Stop` hook gives what waits as the turn ends.
pub fn nudged_unless_socket(session: &mut Session) {
    if !SOCKET {
        session.prompt(NUDGE);
    }
}

/// Fails unless `turn`'s conversation holds `text` as the line of `msg_id` from `from`, by Ada's
/// device, at `own`: where the client writes into the socket ([`SOCKET`]), as one line and with no
/// UUID ([`assert_own_line`]); otherwise as a hook gives it ([`assert_own_by_hook`]).
#[track_caller]
pub fn assert_own_in(turn: &Value, text: &str, msg_id: MessageId, from: &str) {
    match SOCKET {
        true => drop(assert_own_line(&conversation(turn), text, msg_id, from)),
        false => assert_own_by_hook(turn, text, msg_id, from),
    }
}

/// Fails unless `turn`'s conversation holds `text` as the line of `msg_id` from `from`, by Ada's
/// device, at `own`, wherever in a line it starts: Claude Code puts a hook's context after words
/// of its own.
#[track_caller]
pub fn assert_own_by_hook(turn: &Value, text: &str, msg_id: MessageId, from: &str) {
    let line = line_in(&conversation(turn), text);
    assert_from(&line, msg_id, (from, ADA_HOST, OWN));
}

/// Whether the last user message of a turn is a tool's result.
pub fn after_a_tool(body: &Value) -> bool {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let last = messages
        .iter()
        .rev()
        .find(|message| message["role"] == "user");
    let blocks = last.and_then(|message| message["content"].as_array().cloned());
    blocks
        .unwrap_or_default()
        .iter()
        .any(|block| block["type"] == "tool_result")
}

/// A stub that calls `inbox` on a turn that the signal started, and else ends the turn.
fn inbox_on_the_signal(own: &'static str) -> Model {
    Model::start(move |body| {
        let said = last_user_text(body);
        if !after_a_tool(body) && said.contains(INBOX_WORD) && !said.contains(own) {
            Reply::Skald("inbox")
        } else {
            Reply::Done
        }
    })
}

/// Starts a session of `agent` on `surface`, gives it [`START`], and waits until its turn ended:
/// its `SessionStart` hook has run by then, and its `Stop` hook gave nothing more.
pub fn idle_session(solo: &Solo, agent: &str, model: &Model, surface: Surface) -> Option<Session> {
    let claude = claude_code()?;
    let mut session = Session::start(
        &claude,
        solo.machine(),
        &solo.dir(agent),
        agent,
        model,
        surface,
    );
    session.prompt(START);
    session.wait_turn_end(model, START, 1);
    Some(session)
}

/// Runs the person's `skald recv` on `machine` every half second for [`QUIET`]; fails if a run
/// shows `read` of `msg_id`.
#[track_caller]
fn no_read_meanwhile(machine: &Machine, msg_id: MessageId, when: &str) {
    let until = Instant::now() + QUIET;
    while Instant::now() < until {
        let shown = crate::kit::received(machine).shown();
        assert_eq!(read_lines(&shown, msg_id), 0, "{when}: {shown}");
        thread::sleep(Duration::from_millis(500));
    }
}

/// Fails unless the session's output has a response of each hook of `events`, and every response of
/// its hooks left the session nothing: no output; where the client writes into no socket
/// ([`SOCKET`]), the hooks give the messages, so only those of `SessionStart` and `SessionEnd`. The
/// apps' stream ends before Claude Code runs the `SessionEnd` hook, so it has no response of it.
#[track_caller]
pub fn assert_hooks_silent(session: &Session, events: &[&str]) {
    for event in events {
        let responses = session.hook_responses(event);
        assert!(!responses.is_empty(), "no {event}: {}", session.output());
    }
    let quiet: &[&str] = match SOCKET {
        true => &["SessionStart", "PostToolUse", "Stop", "SessionEnd"],
        false => &["SessionStart", "SessionEnd"],
    };
    for event in quiet.iter().copied() {
        for response in session.hook_responses(event) {
            for field in ["output", "stdout"] {
                assert_eq!(response[field], "", "{event}: {response}");
            }
        }
    }
}

/// What api and review say to web; neither is anywhere else.
const OWN_SAYS: &str = "e2e socket test: api asks web to rerun the migration";
const PEER_SAYS: &str = "e2e socket test: review asks web for the deploy log";

/// Bob joins `clienta`. A Claude Code session of web runs in a terminal in `~/work/clientA/web`;
/// it takes the prompt, and its turn ends. api sends web a message: without another prompt, a
/// turn of the session's holds it as its line from api at `own`, and the person's `skald recv`
/// shows `read` of it once. review sends web a message: a turn holds only the signal, which names
/// `inbox`; the stub calls `inbox`, and the request after it holds review's line at `peer`, after
/// a warning line that names data; Bob's `skald recv` shows `read` of it once. The stub got no
/// credential but its placeholder.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn idle_terminal_session_gets_own_text_and_a_peer_s_signal_without_a_prompt() {
    let solo = Solo::new(4_919_451_001, "ada-socket-e2e-terminal");
    let bob = Bob::join(&solo, 4_919_451_002, "bob-socket-e2e-terminal");
    let model = inbox_on_the_signal(OWN_SAYS);
    let Some(mut session) = idle_session(&solo, WEB, &model, Surface::Terminal) else {
        return;
    };

    let own = solo.api_sends_web(OWN_SAYS);
    nudged_unless_socket(&mut session);
    let turn = model.wait_turn_holding(OWN_SAYS, "api's message");
    assert_own_in(&turn, OWN_SAYS, own, &solo.short(API));
    shows_read_once(solo.machine(), own);

    let peer = bob.sends(&solo, WEB, PEER_SAYS);
    if !SOCKET {
        session.wait_turn_end(&model, OWN_SAYS, 2);
    }
    nudged_unless_socket(&mut session);
    let after_inbox = model.wait_turn_holding(PEER_SAYS, "review's message after inbox");
    let signalled = (model.turns().into_iter())
        .find(|body| !after_a_tool(body) && last_user_text(body).contains(INBOX_WORD))
        .expect("a turn the signal started");
    assert_signal(&last_user_text(&signalled), peer, PEER_SAYS, &bob);
    let shown = last_user_text(&after_inbox);
    let line = agent_line_of(&shown, PEER_SAYS);
    let expected = [bob.review(), BOB_HOST.to_owned(), PEER.to_owned()];
    assert_eq!([line.from, line.device, line.level], expected, "{shown}");
    let before = &shown[..shown.find(PEER_SAYS).unwrap_or_default()];
    assert!(
        before.lines().any(|said| holds_word(said, DATA)),
        "no warning line before review's: {shown}"
    );
    shows_read_once(&bob.person.machine, peer);
    session.end();
    model.assert_no_foreign_credential();
}

/// What api says to web on each surface of the apps; none is anywhere else.
const APP_SAYS: &str = "e2e socket test: api asks web in the app";
const BYPASSING_SAYS: &str = "e2e socket test: api asks web in the app bypassing prompts";

/// Sessions of web run as the desktop app and VS Code run Claude Code, one as they do by default
/// and one bypassing permission prompts; each takes the prompt, and its turn ends. api sends web a
/// message while each runs: without another prompt, a turn of the session holds it as its line from
/// api at `own`, and the person's `skald recv` shows `read` of it once. The session exits; its
/// output has a response of its `SessionStart` hook, and no hook's response holds any output.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn idle_app_sessions_get_own_text_without_a_prompt_bypassing_permissions_or_not() {
    let solo = Solo::new(4_919_452_001, "ada-socket-e2e-apps");
    for (surface, says) in [
        (Surface::App, APP_SAYS),
        (Surface::AppBypassing, BYPASSING_SAYS),
    ] {
        let model = Model::start(|_| Reply::Done);
        let Some(mut session) = idle_session(&solo, WEB, &model, surface) else {
            return;
        };

        let own = solo.api_sends_web(says);
        nudged_unless_socket(&mut session);

        let turn = model.wait_turn_holding(says, &format!("{surface:?}: api's message"));
        assert_own_in(&turn, says, own, &solo.short(API));
        shows_read_once(solo.machine(), own);
        assert!(
            session.end(),
            "{surface:?} did not exit: {}",
            session.output()
        );
        assert_hooks_silent(&session, &["SessionStart"]);
        model.assert_no_foreign_credential();
    }
}

/// What api says to web while its tool runs; it is nowhere else.
const BUSY_SAYS: &str = "e2e socket test: api asks web while it sleeps";

/// A session of web runs as the apps run it; on the prompt the stub runs `sleep 8`. While it
/// runs, api sends web a message. The request after the tool's result, in the same turn, holds it
/// as its line from api at `own`; the session's last request holds its text once; the person's
/// `skald recv` shows `read` of it once. The session exits, and no hook's response holds any
/// output: the message went over the socket alone.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn busy_session_gets_own_text_once_after_its_running_tool() {
    let solo = Solo::new(4_919_453_001, "ada-socket-e2e-busy");
    let model = Model::start(|body| {
        if !after_a_tool(body) && last_user_text(body).contains(START) {
            Reply::Bash("sleep 8")
        } else {
            Reply::Done
        }
    });
    let Some(claude) = claude_code() else { return };
    let mut session = Session::start(
        &claude,
        solo.machine(),
        &solo.dir(WEB),
        WEB,
        &model,
        Surface::App,
    );
    session.prompt(START);
    session.wait_turn_holding(&model, START);

    let own = solo.api_sends_web(BUSY_SAYS);

    let turn = model.wait_turn_holding(BUSY_SAYS, "api's message");
    assert!(after_a_tool(&turn), "not after the tool: {turn}");
    assert_own_in(&turn, BUSY_SAYS, own, &solo.short(API));
    shows_read_once(solo.machine(), own);
    assert!(
        session.end(),
        "the session did not exit: {}",
        session.output()
    );
    let last = conversation(model.turns().last().expect("a turn"));
    assert_eq!(last.matches(BUSY_SAYS).count(), 1, "not once: {last}");
    assert_hooks_silent(&session, &["SessionStart", "PostToolUse", "Stop"]);
    model.assert_no_foreign_credential();
}

/// What api says to web as the model fails; it is nowhere else.
const FAILED_SAYS: &str = "e2e socket test: api asks web while the model fails";
/// The person's next prompt.
const NEXT: &str = "e2e socket test: try again";

/// A session of web runs as the apps run it, idle after the prompt. api sends web a message; the
/// stub answers the turn that holds it with an error Claude Code does not retry: the person's
/// `skald recv` shows no `read` of it for a while. The person gives another prompt, and the stub
/// replies: `read` of it once.
///
/// Where the client writes into no socket ([`SOCKET`]), the hooks hand the message over by
/// construction, so the test does not run.
#[cfg(unix)]
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn an_api_error_after_the_message_gives_no_read_until_the_model_answers() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let solo = Solo::new(4_919_454_001, "ada-socket-e2e-api-error");
    let failed = Arc::new(AtomicBool::new(false));
    let failing = Arc::clone(&failed);
    // Claude Code retries the turn: each try fails, until the person's prompt.
    let model = Model::start(move |body| {
        let said = last_user_text(body);
        if said.contains(FAILED_SAYS) && !said.contains(NEXT) {
            failing.store(true, Ordering::SeqCst);
            Reply::Fail
        } else {
            Reply::Done
        }
    });
    let Some(mut session) = idle_session(&solo, WEB, &model, Surface::App) else {
        return;
    };

    let own = solo.api_sends_web(FAILED_SAYS);
    model.wait_turn_holding(FAILED_SAYS, "api's message");
    // The stub records a turn before it answers.
    let until = Instant::now() + QUIET;
    while !failed.load(Ordering::SeqCst) {
        assert!(Instant::now() < until, "the stub did not fail the turn");
        thread::sleep(Duration::from_millis(100));
    }
    no_read_meanwhile(solo.machine(), own, "after the API error");
    session.prompt(NEXT);
    session.wait_turn_end(&model, NEXT, 2);

    shows_read_once(solo.machine(), own);
    session.end();
    model.assert_no_foreign_credential();
}
