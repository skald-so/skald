//! In `pull`, no hook and no `codex queue` gives a channel's messages from one trust level, and no
//! wake counts for them; `inbox` returns them. A message of another level there, or of the level in
//! another channel, still wakes the session at once.

use crate::codex::{Codex, Event};
use crate::delivery::{
    PULL, assert_wakes, bob_joins_other_channel, other_channel, review_sends_in, set_mode, wakes,
};
use crate::modes::{quiet_turn, review_sends, review_session};
use crate::queued::assert_inbox_gives;
use crate::senders::PEER;
use crate::standin::{assert_queued, assert_quiet, assert_soon, web_in_app};

/// What Bob's review says to web in the default channel, in `pull`; none is anywhere else.
const PULLED_IDLE: [&str; 3] = [
    "codex modes test: review asks web, in pull, to lint",
    "codex modes test: review asks web, in pull, to format",
    "codex modes test: review asks web, in pull, to test",
];
const PULLED_BUSY: &str = "codex modes test: review asks web, in pull while busy, to bench";

/// What api says to web in the default channel, and Bob's review in the other; neither is anywhere
/// else.
const OWN_THERE: &str = "codex modes test: api asks web, beside the pulled peer, for the logs";
const PEER_ELSEWHERE: &str = "codex modes test: review asks web, in the other channel, for a fix";

/// Bob joins `clienta` and its other channel; the person sets `peer` in the default channel to
/// `pull`. web's session starts in a Codex app of its own: it is idle. Bob's review sends web three
/// messages there: the app's `codex` does not run. web's turn starts and review sends web another:
/// web's `PostToolUse` and `Stop` print nothing, and the app's `codex` still does not run; the
/// default channel has no wake. api sends web a message there: `codex queue` runs within a second;
/// a turn runs that gives nothing. review sends web a message in the other channel: `codex queue`
/// runs within a second; a turn runs that gives nothing. The app's `codex` ran twice in all; each
/// channel has one wake; web's `inbox` gives the six messages.
pub fn in_pull_no_path_wakes_the_session_and_inbox_returns_every_message() {
    let mut codex = Codex::new(4_929_003_001, "ada-codex-modes-pull");
    let bob = codex.web.with_bob(4_929_003_002, "bob-codex-modes-pull");
    bob_joins_other_channel(&codex.web.solo, &bob);
    let solo = &codex.web.solo;
    let (default, other) = (solo.default_channel(), other_channel(solo));
    set_mode(solo, default, PEER, PULL, None);
    let mut review = review_session(&bob);
    let (app, web, mut process) = web_in_app(&codex);

    let mut sent = Vec::new();
    for text in PULLED_IDLE {
        sent.push((text, review_sends(&codex, &mut review, text)));
    }
    assert_quiet(&[(&app, 0, "codex queue for pulled messages")]);

    process.silent(&codex, &web, Event::UserPromptSubmit, "the turn's start");
    sent.push((PULLED_BUSY, review_sends(&codex, &mut review, PULLED_BUSY)));
    assert_quiet(&[(&app, 0, "codex queue for a pulled message")]);
    process.silent(&codex, &web, Event::PostToolUse, "a pulled message");
    process.silent(
        &codex,
        &web,
        Event::Stop,
        "the turn's end with pulled messages",
    );
    assert_eq!(
        wakes(&codex.web.solo, default),
        0,
        "a pulled message woke it"
    );

    let own = codex.web.own(OWN_THERE);
    let ran = app.await_runs(1);
    assert_soon(&ran[0], &codex, own);
    assert_queued(&ran[0], (&web, &app), &[OWN_THERE], &[own], &[]);
    quiet_turn(&mut process, &codex, &web, "the turn the own message woke");

    let elsewhere = review_sends_in(&codex.web.solo, &bob, other, PEER_ELSEWHERE);
    let ran = app.await_runs(2);
    assert_soon(&ran[1], &codex, elsewhere);
    assert_queued(&ran[1], (&web, &app), &[PEER_ELSEWHERE], &[elsewhere], &[]);
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the other channel woke",
    );

    assert_quiet(&[(&app, 2, "codex queue for pulled messages")]);
    assert_wakes(&codex.web.solo, default, 1);
    assert_wakes(&codex.web.solo, other, 1);
    sent.extend([(OWN_THERE, own), (PEER_ELSEWHERE, elsewhere)]);
    assert_inbox_gives(&mut codex, &sent);
}
