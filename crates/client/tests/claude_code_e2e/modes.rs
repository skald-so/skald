//! The delivery modes on a running Claude Code session that gets messages over its socket: with no
//! setting each message wakes an idle session at once; in `batch` the messages of one window wake
//! it once, at the window's end, all in one turn; in `pull` none wakes it, no hook gives any, and
//! `inbox` returns them all; a mode set for one channel and level leaves the others at `now`. The
//! person's listing gives the wakes per channel. Under `inline` for `peer`, a peer's message reaches
//! an idle session as its text, labelled as data, with no `inbox` call. Where the client writes into no socket, nothing
//! wakes an idle session, so these tests run on Unix only.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::MessageId;

use crate::delivery::{
    BATCH, INLINE, PULL, api_sends_in, api_sends_ten, assert_inline, assert_wakes,
    bob_joins_other_channel, other_channel, review_sends_in, set_mode, set_text, wakes,
};
use crate::hooks::{Bob, INBOX_WORD, assert_signal};
use crate::receipts::shows_read_once;
use crate::senders::{OWN, PEER};
use crate::socket_claude::{Session, Surface};
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::socket_sessions::{
    QUIET, after_a_tool, assert_hooks_silent, assert_own_in, idle_session,
};
use crate::solo::{API, Solo, WEB};

/// How long a test waits for a turn the stub may not have got yet.
const REACH: Duration = Duration::from_secs(120);

/// Waits until a turn of `model` is one `holds` says it is, for up to [`REACH`]; fails saying
/// `what` and what `session` printed. Gives the first such turn.
#[track_caller]
pub fn wait_turn_where(
    model: &Model,
    session: &Session,
    holds: impl Fn(&Value) -> bool,
    what: &str,
) -> Value {
    let deadline = Instant::now() + REACH;
    loop {
        if let Some(turn) = model.turns().into_iter().find(|turn| holds(turn)) {
            return turn;
        }
        assert!(
            Instant::now() < deadline,
            "{what}: no such turn; the session printed: {}",
            session.output()
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Fails if a turn of `model` holds one of `texts`.
#[track_caller]
fn no_turn_holds(model: &Model, texts: &[&str], when: &str) {
    for turn in model.turns() {
        let all = conversation(&turn);
        for text in texts {
            assert!(!all.contains(text), "{when}: a turn holds {text:?}: {all}");
        }
    }
}

/// A session of web on `surface`, on a stub that ends each turn, idle after the prompt; `None`
/// without Claude Code.
fn idle_on_a_quiet_stub(solo: &Solo, surface: Surface) -> Option<(Model, Session)> {
    let model = Model::start(|_| Reply::Done);
    let session = idle_session(solo, WEB, &model, surface)?;
    Some((model, session))
}

/// What api says to web; none is anywhere else.
const THREE: [&str; 3] = [
    "e2e modes test: api asks web to draft the spec",
    "e2e modes test: api asks web to review the spec",
    "e2e modes test: api asks web to publish the spec",
];

/// With no setting, a Claude Code session of web runs in a terminal in `~/work/clientA/web`; it
/// takes the prompt, and its turn ends. api sends web three messages, one at a time: without a
/// prompt, each starts a turn of its own that holds it as its line from api at `own`, and once
/// that turn ended the listing gives one wake more for the default channel: three in the end.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn with_no_setting_each_of_three_messages_wakes_an_idle_session_at_once() {
    let solo = Solo::new(5_207_401_001, "ada-modes-e2e-now");
    let default = solo.default_channel();
    let Some((model, mut session)) = idle_on_a_quiet_stub(&solo, Surface::Terminal) else {
        return;
    };

    for (at, text) in THREE.into_iter().enumerate() {
        let msg_id = solo.api_sends_web(text);

        let turn = model.wait_turn_holding(text, "api's message");
        assert!(
            last_user_text(&turn).contains(text),
            "no turn of its own: {turn}"
        );
        assert_own_in(&turn, text, msg_id, &solo.short(API));
        session.wait_turn_end(&model, text, at + 2);
        assert_wakes(&solo, default, at as u64 + 1);
    }
    session.end();
    model.assert_no_foreign_credential();
}

/// The window of `batch` in the end-to-end tests, as the CLI takes it and as a duration.
const WINDOW: (&str, Duration) = ("20s", Duration::from_secs(20));

/// What each of api's ten messages starts with; nothing else holds it.
const TEN: &str = "e2e modes test: api asks web, in batch, for part";

/// A session of web runs in a terminal, idle after the prompt; the person sets `own` in the default
/// channel to `batch` with a window of 20 seconds. api's MCP session sends web ten messages: until
/// the window's end no turn holds any of them. Then the first turn that holds one of them holds all
/// ten, as their lines from api; the listing gives one wake for the default channel, and Ada's
/// machine shows `read` of the last once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn in_batch_ten_messages_within_one_window_wake_the_session_once() {
    let solo = Solo::new(5_207_402_001, "ada-modes-e2e-batch");
    let default = solo.default_channel();
    let Some((model, mut session)) = idle_on_a_quiet_stub(&solo, Surface::Terminal) else {
        return;
    };
    set_mode(&solo, default, OWN, BATCH, Some(WINDOW.0));
    let start = Instant::now();
    let sent = api_sends_ten(&solo, TEN);

    assert!(
        start.elapsed() < WINDOW.1,
        "the ten sends took longer than the window: {:?}",
        start.elapsed()
    );
    thread::sleep(WINDOW.1.saturating_sub(start.elapsed()));
    no_turn_holds(&model, &[TEN], "within the window");

    let turn = wait_turn_where(
        &model,
        &session,
        |turn| conversation(turn).contains(TEN),
        "api's ten messages",
    );
    for (text, msg_id) in &sent {
        assert_own_in(&turn, text, *msg_id, &solo.short(API));
    }
    session.wait_turn_end(&model, &sent[9].0, 2);
    assert_wakes(&solo, default, 1);
    shows_read_once(solo.machine(), sent[9].1);
    session.end();
    model.assert_no_foreign_credential();
}

/// What api says to web, and the person's prompt that has the stub call `inbox`; none is anywhere
/// else.
const PULLED: [&str; 3] = [
    "e2e modes test: api asks web, in pull, to profile",
    "e2e modes test: api asks web, in pull, to cache",
    "e2e modes test: api asks web, in pull, to measure",
];
const CHECK: &str = "e2e modes test: check your inbox";

/// A session of web runs as the apps run it, idle after the prompt; the person sets `own` in the
/// default channel to `pull`. api sends web three messages: in [`QUIET`] no turn holds any of them.
/// The person gives a prompt, and the stub calls `inbox`: the first turn that holds one of them is
/// after that call and holds all three, as their lines from api. The session exits: no hook's
/// response holds any output, the listing gives no wake for the default channel, and Ada's machine
/// shows `read` of each once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn in_pull_no_message_reaches_the_model_until_inbox_returns_them_all() {
    let solo = Solo::new(5_207_403_001, "ada-modes-e2e-pull");
    let default = solo.default_channel();
    let model = Model::start(|body| {
        if !after_a_tool(body) && last_user_text(body).contains(CHECK) {
            Reply::Skald("inbox")
        } else {
            Reply::Done
        }
    });
    let Some(mut session) = idle_session(&solo, WEB, &model, Surface::App) else {
        return;
    };
    set_mode(&solo, default, OWN, PULL, None);
    let pulled: Vec<MessageId> = PULLED.iter().map(|text| solo.api_sends_web(text)).collect();

    thread::sleep(QUIET);
    no_turn_holds(&model, &PULLED, "before inbox");
    session.prompt(CHECK);

    let turn = model.wait_turn_holding(PULLED[0], "api's messages by inbox");
    assert!(after_a_tool(&turn), "not after inbox: {turn}");
    for (text, msg_id) in PULLED.into_iter().zip(&pulled) {
        assert_own_in(&turn, text, *msg_id, &solo.short(API));
    }
    session.wait_turn_end(&model, CHECK, 2);
    assert!(
        session.end(),
        "the session did not exit: {}",
        session.output()
    );
    assert_hooks_silent(&session, &["SessionStart", "PostToolUse", "Stop"]);
    assert_eq!(wakes(&solo, default), 0, "a pulled message woke it");
    for msg_id in pulled {
        shows_read_once(solo.machine(), msg_id);
    }
    model.assert_no_foreign_credential();
}

/// What review and api say to web; none is anywhere else.
const PEER_PULLED: &str = "e2e modes test: review asks web, in pull, for the traces";
const OWN_THERE: &str = "e2e modes test: api asks web, beside the pulled peer, for the spans";
const PEER_ELSEWHERE: &str =
    "e2e modes test: review asks web, in the default channel, to roll back";

/// Bob joins `clienta` and its other channel; a session of web runs in a terminal, idle after the
/// prompt, and the person sets `peer` in the other channel to `pull`. review sends web a message
/// there: in [`QUIET`] no turn holds the signal or the message. api sends web a message there: a
/// turn holds it as its line from api. review sends web a message in the default channel: a turn
/// starts with the signal alone. The listing gives one wake for each channel.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_mode_set_for_peer_in_one_channel_leaves_own_there_and_peer_elsewhere_at_now() {
    let solo = Solo::new(5_207_404_001, "ada-modes-e2e-pull-a-peer");
    let bob = Bob::join(&solo, 5_207_404_002, "bob-modes-e2e-pulled");
    bob_joins_other_channel(&solo, &bob);
    let (default, other) = (solo.default_channel(), other_channel(&solo));
    let Some((model, mut session)) = idle_on_a_quiet_stub(&solo, Surface::Terminal) else {
        return;
    };
    set_mode(&solo, other, PEER, PULL, None);
    let signalled = |turn: &Value| !after_a_tool(turn) && last_user_text(turn).contains(INBOX_WORD);

    review_sends_in(&solo, &bob, other, PEER_PULLED);
    thread::sleep(QUIET);
    no_turn_holds(&model, &[PEER_PULLED], "a pulled peer message");
    assert!(
        !model.turns().iter().any(signalled),
        "a signal of a pulled peer message"
    );

    let own = api_sends_in(&solo, other, OWN_THERE);
    let turn = model.wait_turn_holding(OWN_THERE, "api's message beside the pulled one");
    assert_own_in(&turn, OWN_THERE, own, &solo.short(API));
    session.wait_turn_end(&model, OWN_THERE, 2);

    let elsewhere = bob.sends(&solo, WEB, PEER_ELSEWHERE);
    let turn = wait_turn_where(&model, &session, signalled, "the signal");
    assert_signal(&last_user_text(&turn), elsewhere, PEER_ELSEWHERE, &bob);

    assert_wakes(&solo, other, 1);
    assert_wakes(&solo, default, 1);
    session.end();
    model.assert_no_foreign_credential();
}

/// What review and api say to web; none is anywhere else.
const INLINE_SAYS: &str = "e2e inline test: review asks web, inline, to tune the pool size";
const OWN_SAYS: &str = "e2e inline test: api asks web, beside inline, to restart the pool";
const SIGNALLED: &str = "e2e inline test: review asks web, in the other channel, for the heap dump";

/// Bob joins `clienta` and its other channel; a session of web runs as the apps run it, idle after
/// the prompt, on a stub that calls no tool; the person sets `peer`'s text in the default channel to
/// `inline`. review sends web a message there: without a prompt, a turn starts with its line from
/// review at `peer`, after a line that names data, and Bob's machine shows `read` of it once. api
/// sends web a message: a turn holds it as its line at `own`. review sends web a message in the
/// other channel: a turn starts with the signal alone.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn inline_peer_text_reaches_an_idle_session_over_the_socket_without_inbox() {
    let solo = Solo::new(5_207_501_001, "ada-inline-e2e-socket");
    let bob = Bob::join(&solo, 5_207_501_002, "bob-inline-e2e-socket");
    bob_joins_other_channel(&solo, &bob);
    let Some((model, mut session)) = idle_on_a_quiet_stub(&solo, Surface::App) else {
        return;
    };
    set_text(&solo, solo.default_channel(), PEER, INLINE);

    let asked = bob.sends(&solo, WEB, INLINE_SAYS);
    let turn = model.wait_turn_holding(INLINE_SAYS, "review's message inline");
    assert_inline(&last_user_text(&turn), INLINE_SAYS, asked, &bob);
    shows_read_once(&bob.person.machine, asked);
    session.wait_turn_end(&model, INLINE_SAYS, 2);

    let own = solo.api_sends_web(OWN_SAYS);
    let turn = model.wait_turn_holding(OWN_SAYS, "api's message");
    assert_own_in(&turn, OWN_SAYS, own, &solo.short(API));
    session.wait_turn_end(&model, OWN_SAYS, 3);

    let elsewhere = review_sends_in(&solo, &bob, other_channel(&solo), SIGNALLED);
    let started = wait_turn_where(
        &model,
        &session,
        |turn| !after_a_tool(turn) && last_user_text(turn).contains(INBOX_WORD),
        "the signal",
    );
    assert_signal(&last_user_text(&started), elsewhere, SIGNALLED, &bob);
    assert!(
        !conversation(&started).contains(SIGNALLED),
        "the other channel's text: {started}"
    );
    session.end();
    model.assert_no_foreign_credential();
}
