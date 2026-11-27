//! An idle Claude Code session that refuses cross-session messages wakes without a prompt by the
//! fallbacks: by Channels where it loads them, by the `asyncRewake` hook where it does not, and by
//! the mod, whose prompt only asks the agent to call `inbox`, without both. A session that does not
//! load the channel loses its event, and the message arrives by the next path after the timeout.
//! An `own` message arrives as its line, a `peer` one only as the signal, whose `inbox` call then
//! gives its text. `read` follows once, and the model's requests hold each message once.
//!
//! The mod is the plugin directory [`MOD_DIR`], which Claude Code loads with `--plugin-dir`; it runs
//! the `skald` on the session's `PATH` in the session's folder, as the agent of the session's hooks
//! when they name none.

use std::path::PathBuf;
use std::thread;

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::claude::claude_code;
use crate::client::CLIENT;
use crate::helped::Helped;
use crate::hooks::{Bob, INBOX_WORD, api_sends, api_sends_web};
use crate::lines::{agent_line_of, agent_lines, reference};
use crate::receipts::shows_read_once;
use crate::senders::OWN;
use crate::socket_claude::{Options, Session, Surface};
use crate::socket_model::{Model, Reply, conversation, last_user_text};
use crate::socket_sessions::{
    QUIET, START, after_a_tool, assert_own_by_hook, assert_signal_then_inbox,
};
use crate::solo::{API, Solo, WEB};
use crate::talk::ADA_HOST;

/// The mod's plugin directory.
pub const MOD_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/claude-mod");

/// The resend timeout of these sessions, in seconds.
const RESEND: &str = "5";

/// How Claude Code puts a channel event of the session's `skald mcp` into the conversation.
const CHANNEL_TAG: &str = "<channel source=\"skald\"";

/// The fallback a session of web loads: its `skald mcp` as a development channel, or the
/// `asyncRewake` hooks without the channel flag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fallback {
    Channels,
    Rewake,
}

impl Fallback {
    /// The session's options for it.
    fn options(self) -> Options {
        refusing(Options {
            channels: self == Fallback::Channels,
            rewake: self == Fallback::Rewake,
            ..Options::default()
        })
    }

    /// Fails unless `turn`, the turn the message started, came by this path: by a channel event
    /// or by none.
    #[track_caller]
    fn assert_came_by(self, turn: &Value) {
        let by_channel = conversation(turn).contains(CHANNEL_TAG);
        assert_eq!(
            by_channel,
            self == Fallback::Channels,
            "by the wrong path: {turn}"
        );
    }
}

/// Ada, GitHub user `id` with `login`, runs a session of web that loads `fallback` and takes the prompt;
/// api sends web `says`. Fails unless, without another prompt, a turn holds it by `fallback` as its line
/// from api at `own`, the person's `skald recv` shows `read` of it once, and a while later the last
/// request holds it once.
fn own_wakes(id: u64, login: &str, fallback: Fallback, says: &str) {
    let solo = Solo::new(id, login);
    let helped = Helped::new(solo.machine());
    let model = Model::start(|_| Reply::Done);
    let Some(mut session) = idle_in_terminal((&solo, &helped), WEB, &model, fallback.options())
    else {
        return;
    };

    let own = api_sends_web(&solo, says);

    let turn = model.wait_turn_holding(says, "api's message");
    fallback.assert_came_by(&turn);
    assert_own_by_hook(&turn, says, own, &solo.short(API));
    shows_read_once(solo.machine(), own);
    held_once_after_a_while(&model, says);
    session.end();
    model.assert_no_foreign_credential();
}

/// Ada, GitHub user `id` with `login`, and Bob, `bob_id`, in `clienta`; a session of web loads
/// `fallback` and takes the prompt; review sends web `says`. Fails unless, without another prompt, a
/// turn holds only the signal, by `fallback`, the stub's `inbox` call then gives review's line at
/// `peer`, and Bob's `skald recv` shows `read` of it once.
fn peer_wakes(
    (id, login): (u64, &str),
    (bob_id, bob_login): (u64, &str),
    fallback: Fallback,
    says: &'static str,
) {
    let solo = Solo::new(id, login);
    let bob = Bob::join(&solo, bob_id, bob_login);
    let helped = Helped::new(solo.machine());
    let model = inbox_when_asked(says);
    let Some(mut session) = idle_in_terminal((&solo, &helped), WEB, &model, fallback.options())
    else {
        return;
    };

    let peer = bob.sends(&solo, WEB, says);

    let signalled = assert_signal_then_inbox(&model, &bob, peer, says);
    fallback.assert_came_by(&signalled);
    shows_read_once(&bob.person.machine, peer);
    session.end();
    model.assert_no_foreign_credential();
}

/// `options` for a session that refuses cross-session messages, `crossSessionInbound` `refuse`,
/// with the resend timeout [`RESEND`].
fn refusing(mut options: Options) -> Options {
    (options.settings).insert("crossSessionInbound".to_owned(), json!("refuse"));
    (options.skald_settings).push(("SKALD_RESEND_AFTER".to_owned(), RESEND.to_owned()));
    options
}

/// Starts a session in a terminal in the folder of `agent`, as `agent` unless `options` say it
/// is unnamed, on `model`, with `options`, its `skald` the copy of `helped`, whose processes it
/// kills on drop, before the test's folder goes; gives it [`START`] and waits until its turn ended.
fn idle_in_terminal(
    (solo, helped): (&Solo, &Helped),
    agent: &str,
    model: &Model,
    options: Options,
) -> Option<Session> {
    let claude = claude_code()?;
    let dir = solo.dir(agent);
    let options = Options {
        skald: Some(helped.exe().to_owned()),
        ..options
    };
    let mut session = Session::start_with(
        &claude,
        (solo.machine(), &dir),
        agent,
        model,
        Surface::Terminal,
        options,
    );
    session.prompt(START);
    session.wait_turn_end(model, START, 1);
    Some(session)
}

/// A stub that calls `inbox` on a turn a prompt or a message that names it started, unless it
/// holds `text`, and else ends the turn.
fn inbox_when_asked(text: &'static str) -> Model {
    Model::start(move |body| {
        let said = last_user_text(body);
        if !after_a_tool(body) && said.contains(INBOX_WORD) && !said.contains(text) {
            Reply::Skald("inbox")
        } else {
            Reply::Done
        }
    })
}

/// Waits for [`QUIET`]; fails unless the session's last request holds `text` once.
#[track_caller]
fn held_once_after_a_while(model: &Model, text: &str) {
    thread::sleep(QUIET);
    let last = conversation(model.turns().last().expect("a turn"));
    assert_eq!(last.matches(text).count(), 1, "not once: {last}");
}

/// The first turn that is not after a tool and whose starting text names `inbox`.
fn asked_for_inbox(model: &Model) -> Value {
    (model.turns().into_iter())
        .find(|body| !after_a_tool(body) && last_user_text(body).contains(INBOX_WORD))
        .expect("a turn that asks for inbox")
}

/// What api says to web in each test; none is anywhere else.
const CHANNEL_SAYS: &str = "e2e wake test: api asks web by its channel";
const REWAKE_SAYS: &str = "e2e wake test: api asks web without a channel";
const MOD_SAYS: &str = "e2e wake test: api asks the unnamed session through the mod";

/// A session of web that refuses cross-session messages runs in a terminal with its `skald mcp`
/// loaded as a development channel; it takes the prompt, and its turn ends. api sends web a
/// message: without another prompt, a turn holds it by the channel, as its line from api at `own`,
/// and the person's `skald recv` shows `read` of it once. A while later the session's last request
/// holds it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn idle_session_gets_own_text_by_channels_without_a_prompt() {
    own_wakes(
        4_919_551_001,
        "ada-wake-e2e-channels",
        Fallback::Channels,
        CHANNEL_SAYS,
    );
}

/// A session of web that refuses cross-session messages runs in a terminal with the `asyncRewake`
/// hooks and its `skald mcp`, but without the channel flag, so it drops the channel's events; it
/// takes the prompt, and its turn ends. api sends web a message: without another prompt, a turn
/// holds it, by the hook and by no channel, as its line from api at `own`, and the person's `skald
/// recv` shows `read` of it once. A while later the session's last request holds it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn idle_session_without_the_channel_flag_gets_own_text_by_async_rewake() {
    own_wakes(
        4_919_552_001,
        "ada-wake-e2e-rewake",
        Fallback::Rewake,
        REWAKE_SAYS,
    );
}

/// The default name of the agent that a session's `skald mcp` registers without `--agent` on
/// Ada's machine: Claude Code's client name, her device's name and its reference.
pub(super) fn default_agent(solo: &Solo) -> String {
    let device = reference(MessageId::from_bytes(*solo.talk.ada.device.as_bytes()));
    format!("{CLIENT}-{ADA_HOST}-{}", &device[..6])
}

/// A session in `~/work/clientA/session` that refuses cross-session messages runs in a terminal
/// with the mod loaded and `skald` on its `PATH`, neither its `skald mcp` nor its hooks given
/// `--agent`, without the channel flag or the `asyncRewake` hooks; it takes the prompt, and its
/// turn ends. api sends the session's agent a message: without another prompt, a turn starts whose
/// prompt names `inbox` and holds neither the message's text nor a message line; the stub calls
/// `inbox`, and the request after it holds the message as its line from api at `own`. The person's
/// `skald recv` shows `read` of it once; a while later the session's last request holds it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn idle_session_without_both_gets_own_text_by_the_mod_s_inbox_prompt() {
    let solo = Solo::new(4_919_553_001, "ada-wake-e2e-mod");
    let helped = Helped::new(solo.machine());
    let plugin = PathBuf::from(MOD_DIR);
    assert!(plugin.is_dir(), "no mod at {}", plugin.display());
    let model = inbox_when_asked(MOD_SAYS);
    let options = refusing(Options {
        plugin_dirs: vec![plugin],
        unnamed: true,
        skald_on_path: true,
        ..Options::default()
    });
    let Some(mut session) = idle_in_terminal((&solo, &helped), "session", &model, options) else {
        return;
    };

    let own = api_sends(&solo, &solo.short(&default_agent(&solo)), MOD_SAYS);

    let after = model.wait_turn_holding(MOD_SAYS, "api's message after the mod's prompt");
    let prompt = last_user_text(&asked_for_inbox(&model));
    assert!(
        !prompt.contains(MOD_SAYS),
        "the text in the mod's prompt: {prompt}"
    );
    assert!(
        agent_lines(&prompt).is_empty(),
        "a message line in the mod's prompt: {prompt}"
    );
    assert!(after_a_tool(&after), "not after inbox: {after}");
    let line = agent_line_of(&last_user_text(&after), MOD_SAYS);
    assert_eq!([line.from, line.level], [solo.short(API), OWN.to_owned()]);
    shows_read_once(solo.machine(), own);
    held_once_after_a_while(&model, MOD_SAYS);
    session.end();
    model.assert_no_foreign_credential();
}

/// What review says to web in each test; none is anywhere else.
const PEER_BY_CHANNEL: &str = "e2e wake test: review asks web by its channel";
const PEER_BY_REWAKE: &str = "e2e wake test: review asks web without a channel";

/// Bob joins `clienta`. A session of web that refuses cross-session messages runs in a terminal
/// with its `skald mcp` loaded as a development channel; it takes the prompt, and its turn ends.
/// review sends web a message: without another prompt, a turn holds only the signal, by the
/// channel; the stub calls `inbox`, and the request after it holds review's line at `peer`. Bob's
/// `skald recv` shows `read` of it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_peer_s_message_wakes_the_session_by_channels_with_the_signal_alone() {
    let ada = (4_919_554_001, "ada-wake-e2e-peer-channel");
    let bob = (4_919_554_002, "bob-wake-e2e-peer-channel");
    peer_wakes(ada, bob, Fallback::Channels, PEER_BY_CHANNEL);
}

/// Bob joins `clienta`. A session of web that refuses cross-session messages runs in a terminal
/// with the `asyncRewake` hooks, without the channel flag; it takes the prompt, and its turn ends.
/// review sends web a message: without another prompt, a turn holds only the signal; the stub
/// calls `inbox`, and the request after it holds review's line at `peer`. Bob's `skald recv` shows
/// `read` of it once.
#[test]
#[ignore = "e2e: runs the real Claude Code, in a job of its own"]
fn a_peer_s_message_wakes_the_session_by_async_rewake_with_the_signal_alone() {
    let ada = (4_919_555_001, "ada-wake-e2e-peer-rewake");
    let bob = (4_919_555_002, "bob-wake-e2e-peer-rewake");
    peer_wakes(ada, bob, Fallback::Rewake, PEER_BY_REWAKE);
}
