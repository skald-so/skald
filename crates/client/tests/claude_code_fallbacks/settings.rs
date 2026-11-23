//! The delivery settings on the fallbacks (`claude_code_modes/main.rs`): Channels, the
//! `asyncRewake` hook and the mod push nothing under `pull`, push a message under `batch` only once
//! its window ends, and give a peer's text under `inline` after the warning line; each push counts
//! one wake for each channel whose messages it carries.

use std::thread;
use std::time::{Duration, Instant};

use crate::cloud::WAIT;
use crate::delivery::{
    BATCH, INLINE, PULL, assert_inline, assert_wakes, set_mode, set_text, wakes,
};
use crate::fallbacks::{Bare, QUIET, RESEND_SETTING, events, session_start, wait_events, woke};
use crate::hooks::{Bob, assert_own_line};
use crate::senders::{OWN, PEER};
use crate::solo::{API, WEB};

/// A resend timeout longer than any test waits: each message stays on its first path.
const LONG: &str = "600";

/// The settings of web's paths in these tests.
const SETTINGS: [(&str, &str); 1] = [(RESEND_SETTING, LONG)];

/// How long the `asyncRewake` hook waits in these tests: well beyond what a test takes.
const WAITS: u64 = 120;

/// The window of `batch` in these tests, as the CLI takes it and as a duration.
const WINDOW: (&str, Duration) = ("8s", Duration::from_secs(8));

/// A wait that a window must have passed in: a second short of it.
const NEARLY: Duration = Duration::from_secs(7);

/// Ada with `own` in the default channel set to `mode`, with `window` for `batch`.
fn with_own(id: u64, login: &str, mode: &str, window: Option<&str>) -> Bare {
    let ada = Bare::new(id, login);
    set_mode(&ada.solo, ada.solo.default_channel(), OWN, mode, window);
    ada
}

/// Fails unless the default channel has no wake.
#[track_caller]
fn no_wake(ada: &Bare) {
    let got = wakes(&ada.solo, ada.solo.default_channel());
    assert_eq!(got, 0, "a pulled message woke the session");
}

/// What api says to web in each test; none is anywhere else.
const CHANNEL_PULLED: &str = "settings test: api asks web, in pull, by Channels";
const CHANNEL_BATCHED: &str = "settings test: api asks web, in batch, by Channels";
const CHANNEL_INLINE: &str = "settings test: review asks web, inline, by Channels";

/// `own` in the default channel is `pull`; web's `skald mcp` runs. api sends web a message: over
/// [`QUIET`] no channel event comes, and the default channel has no wake.
#[test]
fn channels_push_nothing_under_pull() {
    let ada = with_own(4_919_531_001, "ada-settings-channel-pull", PULL, None);
    let mut session = ada.channel(WEB, &SETTINGS);

    ada.solo.api_sends_web(CHANNEL_PULLED);

    let sent = events(&mut session, 1, QUIET);
    assert!(sent.is_empty(), "events under pull: {sent:#?}");
    no_wake(&ada);
}

/// `own` in the default channel is `batch` with a window of 8 seconds; web's `skald mcp` runs. api
/// sends web a message: no channel event comes within 7 seconds; then one, the message's line from
/// api at `own`, and the default channel has one wake.
#[test]
fn channels_push_under_batch_once_the_window_ends() {
    let ada = with_own(
        4_919_531_002,
        "ada-settings-channel-batch",
        BATCH,
        Some(WINDOW.0),
    );
    let mut session = ada.channel(WEB, &SETTINGS);

    let msg_id = ada.solo.api_sends_web(CHANNEL_BATCHED);
    let sent_at = Instant::now();

    let early = events(&mut session, 1, NEARLY.saturating_sub(sent_at.elapsed()));
    assert!(early.is_empty(), "an event within the window: {early:#?}");
    let sent = wait_events(&mut session, 1, WAIT);
    assert_own_line(&sent[0], CHANNEL_BATCHED, msg_id, &ada.solo.short(API));
    assert_wakes(&ada.solo, ada.solo.default_channel(), 1);
}

/// Bob joins `clienta`; `peer`'s text in the default channel is `inline`; web's `skald mcp` runs.
/// review sends web a message: one channel event comes, the message's line at `peer` after a line
/// that names data, and the default channel has one wake.
#[test]
fn channels_give_an_inline_peer_s_text_after_the_warning_line() {
    let ada = Bare::new(4_919_531_003, "ada-settings-channel-inline");
    let bob = Bob::join(&ada.solo, 4_919_531_004, "bob-settings-channel-inline");
    set_text(&ada.solo, ada.solo.default_channel(), PEER, INLINE);
    let mut session = ada.channel(WEB, &SETTINGS);

    let msg_id = bob.sends(&ada.solo, WEB, CHANNEL_INLINE);

    let sent = wait_events(&mut session, 1, WAIT);
    assert_inline(&sent[0], CHANNEL_INLINE, msg_id, &bob);
    assert_wakes(&ada.solo, ada.solo.default_channel(), 1);
}

/// What api and review say to web in each test; none is anywhere else.
const REWAKE_PULLED: &str = "settings test: api asks web, in pull, by asyncRewake";
const REWAKE_BATCHED: &str = "settings test: api asks web, in batch, by asyncRewake";
const REWAKE_INLINE: &str = "settings test: review asks web, inline, by asyncRewake";

/// `own` in the default channel is `pull`; web's `asyncRewake` hook starts on `SessionStart`. api
/// sends web a message: over [`QUIET`] the hook still runs, and the default channel has no wake.
#[test]
fn async_rewake_wakes_nothing_under_pull() {
    let ada = with_own(4_919_532_001, "ada-settings-rewake-pull", PULL, None);
    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &SETTINGS);

    ada.solo.api_sends_web(REWAKE_PULLED);

    thread::sleep(QUIET);
    rewake.assert_running("under pull");
    no_wake(&ada);
}

/// `own` in the default channel is `batch` with a window of 8 seconds; web's `asyncRewake` hook
/// starts on `SessionStart`. api sends web a message: the hook still runs 7 seconds later; then it
/// exits with 2, its standard error the message's line from api at `own`, and the default channel
/// has one wake.
#[test]
fn async_rewake_wakes_under_batch_once_the_window_ends() {
    let ada = with_own(
        4_919_532_002,
        "ada-settings-rewake-batch",
        BATCH,
        Some(WINDOW.0),
    );
    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &SETTINGS);

    let msg_id = ada.solo.api_sends_web(REWAKE_BATCHED);
    let sent_at = Instant::now();

    thread::sleep(NEARLY.saturating_sub(sent_at.elapsed()));
    rewake.assert_running("within the window");
    let (hooked, _) = rewake.finished();
    assert_own_line(&woke(&hooked), REWAKE_BATCHED, msg_id, &ada.solo.short(API));
    assert_wakes(&ada.solo, ada.solo.default_channel(), 1);
}

/// Bob joins `clienta`; `peer`'s text in the default channel is `inline`; web's `asyncRewake` hook
/// starts on `SessionStart`. review sends web a message: the hook exits with 2, its standard error
/// the message's line at `peer` after a line that names data, and the default channel has one
/// wake.
#[test]
fn async_rewake_gives_an_inline_peer_s_text_after_the_warning_line() {
    let ada = Bare::new(4_919_532_003, "ada-settings-rewake-inline");
    let bob = Bob::join(&ada.solo, 4_919_532_004, "bob-settings-rewake-inline");
    set_text(&ada.solo, ada.solo.default_channel(), PEER, INLINE);
    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &SETTINGS);

    let msg_id = bob.sends(&ada.solo, WEB, REWAKE_INLINE);

    let (hooked, _) = rewake.finished();
    assert_inline(&woke(&hooked), REWAKE_INLINE, msg_id, &bob);
    assert_wakes(&ada.solo, ada.solo.default_channel(), 1);
}

/// What api says to web in each test; none is anywhere else.
const MOD_PULLED: &str = "settings test: api asks web, in pull, by the mod";
const MOD_BATCHED: &str = "settings test: api asks web, in batch, by the mod";

/// `own` in the default channel is `pull`; web's mod polls. api sends web a message: over
/// [`QUIET`] no poll gives a prompt, though they say one message waits, and the default channel has
/// no wake.
#[test]
fn the_mod_prompts_nothing_under_pull() {
    let ada = with_own(4_919_533_001, "ada-settings-mod-pull", PULL, None);
    ada.mod_polls(WEB, &SETTINGS);

    ada.solo.api_sends_web(MOD_PULLED);

    let prompt = ada.mod_prompt(WEB, &SETTINGS, QUIET);
    assert_eq!(prompt, None, "a prompt under pull");
    assert_eq!(ada.mod_polls(WEB, &SETTINGS)["waiting"], 1);
    no_wake(&ada);
}

/// `own` in the default channel is `batch` with a window of 8 seconds; web's mod polls. api sends
/// web a message: no poll gives a prompt within 7 seconds; then one does, and the default channel
/// has one wake.
#[test]
fn the_mod_prompts_under_batch_once_the_window_ends() {
    let ada = with_own(
        4_919_533_002,
        "ada-settings-mod-batch",
        BATCH,
        Some(WINDOW.0),
    );
    ada.mod_polls(WEB, &SETTINGS);

    ada.solo.api_sends_web(MOD_BATCHED);
    let sent_at = Instant::now();

    let early = ada.mod_prompt(WEB, &SETTINGS, NEARLY.saturating_sub(sent_at.elapsed()));
    assert_eq!(early, None, "a prompt within the window");
    let prompt = ada.mod_prompt(WEB, &SETTINGS, WAIT);
    assert!(prompt.is_some(), "no prompt once the window ended");
    assert_wakes(&ada.solo, ada.solo.default_channel(), 1);
}
