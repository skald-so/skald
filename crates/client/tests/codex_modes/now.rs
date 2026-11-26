//! With no setting, `now`: each message wakes a Codex session at once, an idle one by `codex queue`,
//! a busy one by its next hook, and each wake counts one for each channel whose messages it covers.

use crate::codex::{Codex, Event, assert_bare};
use crate::delivery::{api_sends_in, assert_wakes, other_channel, wakes};
use crate::modes::{assert_shown_wakes, quiet_turn};
use crate::queued::assert_inbox_gives;
use crate::standin::{assert_queued, assert_quiet, assert_soon, now, web_in_app};

/// What api says to web; none is anywhere else.
const BUSY_HERE: &str = "codex modes test: api asks web, busy, to fix the build";
const BUSY_THERE: &str = "codex modes test: api asks web, busy, in the other channel, for logs";
const IDLE_HERE: &str = "codex modes test: api asks web, idle, to tag the release";
const IDLE_THERE: &str = "codex modes test: api asks web, idle, in the other channel, for notes";

/// web's session starts in a Codex app of its own, and its turn starts. api sends web a message in
/// the default channel and one in the other: the app's `codex` does not run, web's `PostToolUse`
/// gives the signal once, and the listing gives one wake for each channel. web's `Stop` ends the
/// turn. api sends web another message in the default channel: the app's `codex` runs `codex
/// queue` within a second, and the default channel has two wakes, the other one; a turn runs that
/// gives nothing. api sends one in the other channel: `codex queue` again within a second, and the
/// other channel has two wakes. web's `inbox` gives the four messages and the wakes stay two each,
/// as the plain listing shows; the app's `codex` ran twice in all.
pub fn with_no_setting_each_message_wakes_the_session_at_once_and_counts_a_wake_per_channel() {
    let mut codex = Codex::new(4_929_001_001, "ada-codex-modes-now");
    let solo = &codex.web.solo;
    let (default, other) = (solo.default_channel(), other_channel(solo));
    let (app, web, mut process) = web_in_app(&codex);
    process.silent(&codex, &web, Event::UserPromptSubmit, "the turn's start");

    let busy_here = codex.web.own(BUSY_HERE);
    let busy_there = api_sends_in(&codex.web.solo, other, BUSY_THERE);
    assert_quiet(&[(&app, 0, "codex queue for a busy session")]);
    let signal = process.gives(&codex, &web, Event::PostToolUse);
    let texts = [BUSY_HERE, BUSY_THERE];
    assert_bare(&signal, &texts, &[busy_here, busy_there], &[]);
    assert_wakes(&codex.web.solo, default, 1);
    assert_wakes(&codex.web.solo, other, 1);
    process.silent(&codex, &web, Event::Stop, "the turn's end");

    let idle_here = codex.web.own(IDLE_HERE);
    let came = now();
    let ran = app.await_runs(1);
    assert_soon(&ran[0], came);
    assert_queued(&ran[0], (&web, &app), &[IDLE_HERE], &[idle_here], &[]);
    assert_wakes(&codex.web.solo, default, 2);
    assert_eq!(wakes(&codex.web.solo, other), 1, "the other channel woke");
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the default channel woke",
    );

    let idle_there = api_sends_in(&codex.web.solo, other, IDLE_THERE);
    let came = now();
    let ran = app.await_runs(2);
    assert_soon(&ran[1], came);
    assert_queued(&ran[1], (&web, &app), &[IDLE_THERE], &[idle_there], &[]);
    assert_wakes(&codex.web.solo, other, 2);
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the other channel woke",
    );

    assert_inbox_gives(
        &mut codex,
        &[
            (BUSY_HERE, busy_here),
            (BUSY_THERE, busy_there),
            (IDLE_HERE, idle_here),
            (IDLE_THERE, idle_there),
        ],
    );
    assert_quiet(&[(&app, 2, "one codex queue per idle wake")]);
    for channel in [default, other] {
        assert_eq!(wakes(&codex.web.solo, channel), 2, "inbox woke {channel}");
        assert_shown_wakes(&codex, channel, 2);
    }
}
