//! In `batch`, a channel's messages from one trust level wake a Codex session once per window, at
//! its end; a message of another level there, or of the level in another channel, wakes it at once.

use std::path::Path;
use std::time::{Duration, SystemTime};

use skald_proto::MessageId;

use crate::codex::Codex;
use crate::delivery::{
    BATCH, assert_wakes, bob_joins_other_channel, other_channel, review_sends_in, sends_in,
    set_mode, wakes,
};
use crate::hooks::REVIEW;
use crate::machine::Machine;
use crate::modes::{quiet_turn, review_sends, review_session};
use crate::queued::{assert_inbox_gives, second_device};
use crate::senders::PEER;
use crate::standin::{assert_queued, assert_quiet, assert_soon, now, web_in_app};
use crate::talk::BOB_HOST;

/// The window of `batch`, as the CLI takes it and as a duration: long enough for the messages and
/// the turns within it.
const WINDOW: (&str, Duration) = ("30s", Duration::from_secs(30));

/// How late after its window's end the batch's `codex queue` may start: the message's way to Ada's
/// machine after its send returned, and the helper's own pace.
const LATE: Duration = Duration::from_secs(3);

/// What the agent review of Ada's second machine says to web, at `own`, and what Bob's review says
/// to web in the other channel; neither is anywhere else.
const FROM_LAPTOP: &str = "codex modes test: the laptop asks web, beside the batch, to deploy";
const ELSEWHERE: &str = "codex modes test: review asks web, in the other channel, to profile";

/// The text of the message `n` of ten that Bob's review sends web in the window.
fn batched(n: usize) -> String {
    format!("codex modes test: review's note {n:02} of ten in one window")
}

/// The agent review of Ada's second `machine` sends web `text` from its `folder` in `clienta`'s
/// default channel; gives its `msg_id`.
#[track_caller]
fn laptop_sends(codex: &Codex, (machine, folder): (&Machine, &Path), text: &str) -> MessageId {
    let solo = &codex.web.solo;
    sends_in(
        solo,
        (machine, folder, REVIEW),
        solo.default_channel(),
        text,
    )
}

/// Bob joins `clienta` and its other channel, and Ada's second machine is linked; the person sets
/// `peer` in the default channel to `batch` with a window of 30 seconds. web's session starts in a
/// Codex app of its own: it is idle. Bob's review sends web ten messages in the default channel:
/// the app's `codex` does not run. The second machine's review sends web a message there: `codex
/// queue` runs within a second; a turn runs that gives nothing. Bob's review sends web a message in
/// the other channel: `codex queue` runs within a second; a turn runs that gives nothing. All this
/// within the window. At its end, and not before, `codex queue` runs once more, holding none of
/// the ten; a turn runs that gives nothing. The app's `codex` ran three times in all, each for web's
/// thread with only the signal; the default channel has two wakes, the other one; web's `inbox`
/// gives the twelve messages.
pub fn in_batch_ten_peer_messages_within_a_window_wake_the_session_once_at_its_end() {
    let mut codex = Codex::new(4_929_002_001, "ada-codex-modes-batch");
    let bob = codex.web.with_bob(4_929_002_002, "bob-codex-modes-batch");
    bob_joins_other_channel(&codex.web.solo, &bob);
    let (laptop, folder) = second_device(&codex);
    let solo = &codex.web.solo;
    let (default, other) = (solo.default_channel(), other_channel(solo));
    set_mode(solo, default, PEER, BATCH, Some(WINDOW.0));
    let mut review = review_session(&bob);
    let (app, web, mut process) = web_in_app(&codex);
    let (handle, names) = (bob.person.handle.to_string(), bob.review());
    let names = [names.as_str(), handle.as_str(), BOB_HOST];

    let opened = SystemTime::now();
    let mut texts = Vec::new();
    let mut msg_ids = Vec::new();
    for n in 1..=10 {
        let text = batched(n);
        msg_ids.push(review_sends(&codex, &mut review, &text));
        texts.push(text);
    }
    assert_quiet(&[(&app, 0, "codex queue within the window")]);

    let from_laptop = laptop_sends(&codex, (&laptop.machine, &folder), FROM_LAPTOP);
    let came = now();
    let ran = app.await_runs(1);
    assert_soon(&ran[0], came);
    assert_queued(&ran[0], (&web, &app), &[FROM_LAPTOP], &[from_laptop], &[]);
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the laptop's message woke",
    );

    let elsewhere = review_sends_in(&codex.web.solo, &bob, other, ELSEWHERE);
    let came = now();
    let ran = app.await_runs(2);
    assert_soon(&ran[1], came);
    assert_queued(&ran[1], (&web, &app), &[ELSEWHERE], &[elsewhere], &names);
    quiet_turn(
        &mut process,
        &codex,
        &web,
        "the turn the other channel woke",
    );

    let taken = opened.elapsed().unwrap_or_default();
    assert!(
        taken < WINDOW.1,
        "the messages and turns took longer than the window: {taken:?}"
    );

    let ran = app.await_runs(3);
    let at_end = &ran[2];
    let all: Vec<&str> = texts.iter().map(String::as_str).collect();
    assert_queued(at_end, (&web, &app), &all, &msg_ids, &names);
    let after = (at_end.started().duration_since(opened)).unwrap_or_default();
    assert!(
        after >= WINDOW.1,
        "the batch woke the session {after:?} after its first message, within the window"
    );
    assert!(
        after <= WINDOW.1 + LATE,
        "the batch woke the session {after:?} after its first message, past its end"
    );
    quiet_turn(&mut process, &codex, &web, "the turn the batch woke");

    assert_quiet(&[(&app, 3, "one codex queue for the batch")]);
    assert_wakes(&codex.web.solo, default, 2);
    assert_eq!(
        wakes(&codex.web.solo, other),
        1,
        "the other channel's wakes"
    );
    let mut sent: Vec<(&str, MessageId)> = all.iter().copied().zip(msg_ids).collect();
    sent.extend([(FROM_LAPTOP, from_laptop), (ELSEWHERE, elsewhere)]);
    assert_inbox_gives(&mut codex, &sent);
}
