//! What follows a `codex queue`: the turn its signal starts gets no second signal from its hooks,
//! and a failed one runs no more for the same messages.

use crate::codex::{Codex, Event, assert_bare};
use crate::queued::assert_inbox_gives;
use crate::solo::API;
use crate::standin::{assert_quiet, web_in_app};

/// What api says to web, whose signal is queued; it is nowhere else.
const QUEUED: &str = "codex wake test: api asks web, woken, to rerun the migrations";

/// web's session starts in a Codex app of its own: it is idle. api sends web a message: the app's
/// `codex` runs `codex queue` once. The turn it starts runs: its `UserPromptSubmit` and `Stop`
/// print nothing, as the queued signal gave the message's. web's `inbox` gives the message.
pub fn the_turn_a_queued_signal_starts_gets_no_second_signal_from_its_hooks() {
    let mut codex = Codex::new(4_928_007_001, "ada-codex-wake-woken");
    let (app, web, mut process) = web_in_app(&codex);

    let queued = codex.web.own(QUEUED);
    app.await_runs(1);
    process.silent(
        &codex,
        &web,
        Event::UserPromptSubmit,
        "the woken turn's start",
    );
    process.silent(&codex, &web, Event::Stop, "the woken turn's end");
    assert_quiet(&[(&app, 1, "codex queue after the woken turn")]);
    assert_inbox_gives(&mut codex, &[(QUEUED, queued)]);
}

/// What api says to web while `codex queue` fails, and what web then says to api; neither is
/// anywhere else.
const UNQUEUED: &str = "codex wake test: api asks web, unqueued, to rotate the logs";
const TO_API: &str = "codex wake test: web tells api the logs wait";

/// web's session starts in a Codex app of its own whose `codex` fails each run: it is idle. api
/// sends web a message: the app's `codex` runs once and fails. web sends api a message, which
/// brings web's session nothing new: the app's `codex` does not run again. The person's next
/// prompt gives web's session the signal.
pub fn a_failed_codex_queue_runs_no_more_for_the_same_messages() {
    let mut codex = Codex::new(4_928_008_001, "ada-codex-wake-unqueued");
    let _api_mcp = codex.web.solo.session(API);
    let (app, web, mut process) = web_in_app(&codex);
    app.fail();

    let unqueued = codex.web.own(UNQUEUED);
    let ran = app.await_runs(1);
    assert!(ran[0].failed(), "the stand-in did not fail: {ran:#?}");
    let solo = &codex.web.solo;
    let to = solo.short(API);
    solo.sends(&mut codex.web.session, &to, TO_API);
    assert_quiet(&[(&app, 1, "codex queue again for the same message")]);

    let signal = process.gives(&codex, &web, Event::UserPromptSubmit);
    assert_bare(&signal, &[UNQUEUED], &[unqueued], &[]);
}
