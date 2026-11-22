//! A `codex queue` that fails put nothing into the session, so it counts no wake; the hook that
//! then gives the signal counts one.

use crate::codex::{Codex, Event, assert_bare};
use crate::delivery::{assert_wakes, wakes};
use crate::standin::{assert_queued, assert_quiet, web_in_app};

/// What api says to web; it is nowhere else.
const UNQUEUED: &str = "codex modes test: api asks web, whose codex queue fails, to rotate logs";

/// web's session starts in a Codex app of its own, whose `codex` fails: it is idle. api sends web a
/// message: the app's `codex` runs `codex queue` once and fails, and the default channel has no
/// wake. The person's next prompt starts a turn: its `UserPromptSubmit` gives the signal, and the
/// default channel has one wake, and still one after a while.
pub fn a_failed_codex_queue_counts_no_wake_and_the_next_hook_counts_one() {
    let codex = Codex::new(4_929_006_001, "ada-codex-modes-failed");
    let default = codex.web.solo.default_channel();
    let (app, web, mut process) = web_in_app(&codex);
    app.fail();

    let unqueued = codex.web.own(UNQUEUED);
    let ran = app.await_runs(1);
    assert!(ran[0].failed(), "the stand-in did not fail: {ran:#?}");
    assert_queued(&ran[0], (&web, &app), &[UNQUEUED], &[unqueued], &[]);
    assert_quiet(&[(&app, 1, "codex queue again for the same message")]);
    assert_eq!(
        wakes(&codex.web.solo, default),
        0,
        "a failed codex queue woke it"
    );

    let signal = process.gives(&codex, &web, Event::UserPromptSubmit);
    assert_bare(&signal, &[UNQUEUED], &[unqueued], &[]);
    assert_wakes(&codex.web.solo, default, 1);
    process.silent(&codex, &web, Event::Stop, "the turn's end");
    assert_quiet(&[(&app, 1, "codex queue after the hook gave the signal")]);
    assert_eq!(wakes(&codex.web.solo, default), 1, "the hook's wake");
}
