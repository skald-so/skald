//! `inline` changes nothing on Codex's paths: its hooks and `codex queue` give a peer's message
//! only as the signal, and each counts a wake.

use crate::codex::{Codex, Event, assert_bare};
use crate::delivery::{INLINE, assert_wakes, set_text};
use crate::modes::quiet_turn;
use crate::senders::PEER;
use crate::solo::assert_got;
use crate::standin::{assert_queued, assert_quiet, web_in_app};
use crate::talk::BOB_HOST;

/// What Bob's review says to web, busy and idle; neither is anywhere else.
const WHILE_BUSY: &str = "codex modes test: review asks web, inline and busy, to pin the crate";
const WHILE_IDLE: &str = "codex modes test: review asks web, inline and idle, to vendor it";

/// Bob joins `clienta`; the person sets `peer`'s text in the default channel to `inline`. web's
/// session starts in a Codex app of its own, and its turn starts. Bob's review sends web a message:
/// web's `PostToolUse` gives the signal alone, with none of its text, sender or reference, and
/// `Stop` ends the turn. review sends web another: the app's `codex` runs `codex queue` once, with
/// the signal alone; a turn runs that gives nothing. The default channel has two wakes, and web's
/// `inbox` gives both messages at `peer`.
pub fn inline_for_peer_still_gives_the_codex_session_only_the_signal() {
    let mut codex = Codex::new(4_929_004_001, "ada-codex-modes-inline");
    let bob = codex.web.with_bob(4_929_004_002, "bob-codex-modes-inline");
    let solo = &codex.web.solo;
    let default = solo.default_channel();
    set_text(solo, default, PEER, INLINE);
    let (handle, review) = (bob.person.handle.to_string(), bob.review());
    let names = [review.as_str(), handle.as_str(), BOB_HOST];
    let (app, web, mut process) = web_in_app(&codex);
    process.silent(&codex, &web, Event::UserPromptSubmit, "the turn's start");

    let busy = codex.web.peer(&bob, WHILE_BUSY);
    let signal = process.gives(&codex, &web, Event::PostToolUse);
    assert_bare(&signal, &[WHILE_BUSY], &[busy], &names);
    process.silent(&codex, &web, Event::Stop, "the turn's end");
    assert_quiet(&[(&app, 0, "codex queue for a busy session")]);

    let idle = codex.web.peer(&bob, WHILE_IDLE);
    let ran = app.await_runs(1);
    assert_queued(&ran[0], (&web, &app), &[WHILE_IDLE], &[idle], &names);
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the inline message woke",
    );

    assert_quiet(&[(&app, 1, "one codex queue")]);
    assert_wakes(&codex.web.solo, default, 2);
    let shown = codex.web.session.inbox();
    for (text, msg_id) in [(WHILE_BUSY, busy), (WHILE_IDLE, idle)] {
        assert_got(&shown, text, msg_id, (&review, BOB_HOST, PEER));
    }
}
