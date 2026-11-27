//! A busy Codex session gets no `codex queue`: its hooks give the signal; once its turn ends, it is
//! idle, and the next message is queued.

use crate::codex::{Codex, Event, assert_bare};
use crate::standin::{assert_queued, assert_quiet, assert_soon, web_in_app};

/// What api says to web while its session is busy, and once its turn ended; neither is anywhere
/// else.
const WHILE_BUSY: &str = "codex wake test: api asks web, busy, to update the lockfile";
const AFTER_TURN: &str = "codex wake test: api asks web, idle again, to bump the version";

/// web's session starts in a Codex app of its own, and its turn starts: `UserPromptSubmit` prints
/// nothing. api sends web a message: the app's `codex` does not run, and web's `PostToolUse` gives
/// the signal. web's `Stop` prints nothing and ends the turn. api sends web another message: the
/// app's `codex` runs `codex queue` once, within a second, for web's thread, holding the same line
/// as `PostToolUse` gave and neither message's text; it does not run again.
pub fn a_busy_session_gets_the_signal_from_its_hooks_and_a_queue_once_its_turn_ends() {
    let codex = Codex::new(4_928_002_001, "ada-codex-wake-busy");
    let (app, web, mut process) = web_in_app(&codex);
    process.silent(&codex, &web, Event::UserPromptSubmit, "the turn's start");

    let busy = codex.web.own(WHILE_BUSY);
    assert_quiet(&[(&app, 0, "codex queue for a busy session")]);
    let signal = process.gives(&codex, &web, Event::PostToolUse);
    assert_bare(&signal, &[WHILE_BUSY], &[busy], &[]);
    process.silent(&codex, &web, Event::Stop, "the turn's end");

    let idle = codex.web.own(AFTER_TURN);
    let ran = app.await_runs(1);
    assert_soon(&ran[0], &codex, idle);
    let texts = [WHILE_BUSY, AFTER_TURN];
    let queued = assert_queued(&ran[0], (&web, &app), &texts, &[busy, idle], &[]);
    assert_eq!(
        queued, signal,
        "codex queue got another line than the hooks give"
    );
    assert_quiet(&[(&app, 1, "one codex queue, for the message after the turn")]);
}
