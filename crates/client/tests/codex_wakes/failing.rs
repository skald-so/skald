//! A `codex queue` that fails leaves the messages to the session's hooks and `inbox`.

use crate::codex::{Codex, Event, assert_bare};
use crate::queued::assert_inbox_gives;
use crate::standin::{assert_queued, web_in_app};

/// What api says to web while `codex queue` fails; neither is anywhere else.
const FIRST: &str = "codex wake test: api asks web, with codex queue failing, to retry the job";
const SECOND: &str = "codex wake test: api asks web, still failing, to page the on-call";

/// web's session starts in a Codex app of its own whose `codex` fails each run: it is idle. api
/// sends web a message: the app's `codex` runs `codex queue` for web's thread and fails. api sends
/// web another. The person's next prompt starts a turn: its `UserPromptSubmit` gives the signal,
/// holding neither message's text, and web's `inbox` gives both messages.
pub fn with_codex_queue_failing_the_next_hook_gives_the_signal_and_inbox_every_message() {
    let mut codex = Codex::new(4_928_006_001, "ada-codex-wake-failing");
    let (app, web, mut process) = web_in_app(&codex);
    app.fail();

    let first = codex.web.own(FIRST);
    let ran = app.await_some();
    assert!(ran[0].failed(), "the stand-in did not fail: {ran:#?}");
    assert_queued(&ran[0], (&web, &app), &[FIRST], &[first], &[]);
    let second = codex.web.own(SECOND);

    let signal = process.gives(&codex, &web, Event::UserPromptSubmit);
    assert_bare(&signal, &[FIRST, SECOND], &[first, second], &[]);
    for ran in app.runs() {
        assert_queued(&ran, (&web, &app), &[FIRST, SECOND], &[first, second], &[]);
    }
    assert_inbox_gives(&mut codex, &[(FIRST, first), (SECOND, second)]);
}
