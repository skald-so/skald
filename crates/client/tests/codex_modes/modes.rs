//! What the tests of the delivery settings on Codex sessions share: a turn that must give nothing,
//! Bob's agent review sending over its MCP session, and the wakes the person's plain listing shows.

use skald_proto::{ChannelId, MessageId};

use crate::client::Session;
use crate::codex::{Codex, Event, Thread};
use crate::delivery::{LIST, WAKES, person_sets};
use crate::hooks::{Bob, REVIEW};
use crate::senders::holds_word;
use crate::solo::{WEB, session_in};
use crate::standin::CodexApp;

/// A turn of `of` runs, as one a queued signal starts or the person's prompt does: its
/// `UserPromptSubmit` and its `Stop` print nothing, so no message whose signal did not go is due.
#[track_caller]
pub fn quiet_turn(process: &mut CodexApp, codex: &Codex, of: &Thread, what: &str) {
    process.silent(codex, of, Event::UserPromptSubmit, what);
    process.silent(codex, of, Event::Stop, what);
}

/// Bob's agent review's MCP session, `skald --agent review mcp` in its folder on Bob's machine.
pub fn review_session(bob: &Bob) -> Session {
    session_in(&bob.person.machine, &bob.dir, REVIEW)
}

/// review's MCP session `review` sends web `text` in `clienta`'s default channel; gives its
/// `msg_id`.
#[track_caller]
pub fn review_sends(codex: &Codex, review: &mut Session, text: &str) -> MessageId {
    let solo = &codex.web.solo;
    solo.sends(review, &solo.short(WEB), text)
}

/// Fails unless the person's plain `skald delivery-modes` shows, for `channel`, one line that holds
/// its ID, the word `wakes` and `count`.
#[track_caller]
pub fn assert_shown_wakes(codex: &Codex, channel: ChannelId, count: u64) {
    let shown = person_sets(&codex.web.solo, &[LIST]).shown();
    let (id, count) = (channel.to_string(), count.to_string());
    let lines: Vec<&str> = (shown.lines())
        .filter(|line| line.contains(&id) && holds_word(line, WAKES))
        .collect();
    match lines.as_slice() {
        [line] if holds_word(line, &count) => {}
        _ => panic!("no line of {id} with {count} {WAKES}: {shown}"),
    }
}
