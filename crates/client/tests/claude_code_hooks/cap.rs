//! Claude Code ends a turn after 8 `Stop` continuations in a row with no tool call between them and
//! drops the 9th `Stop` hook's output (`docs/dev/agent-delivery.md`, "Claude Code hooks"): at that
//! cap the hook leaves its messages waiting.

use skald_proto::MessageId;

use crate::hooks::{Event, Web, assert_own_line};
use crate::lines::assert_handed_line;

/// How many `Stop` continuations in a row Claude Code takes.
const CAP: usize = 8;

/// The text of api's `n`th message to web; each is nowhere else.
fn note(n: usize) -> String {
    format!("cap test: api's note {n} to web")
}

/// api sends web a message, then web's hook runs on `event`; fails unless it gives that message as
/// its line from api at `own`.
#[track_caller]
fn continues_with_a_new_message(web: &Web, n: usize, event: Event) -> MessageId {
    let text = note(n);
    let msg_id = web.own(&text);
    let context = web.hook(event).context(event);
    assert_own_line(&context, &text, msg_id, &web.api());
    msg_id
}

/// Before each `Stop` of one turn, api sends web a message. The first `Stop` and the next 7, each in
/// the turn the one before kept going, give their message; the 9th prints nothing, and its message
/// waits: web's `inbox` shows it. In the next turn, the first `Stop` gives a new message again.
#[test]
fn ninth_stop_in_a_row_prints_nothing_and_leaves_its_message_waiting() {
    let mut web = Web::new(4_913_010_001, "ada-hooks-to-the-cap");
    continues_with_a_new_message(&web, 1, Event::Stop);
    for n in 2..=CAP {
        continues_with_a_new_message(&web, n, Event::StopActive);
    }

    let last = note(CAP + 1);
    let waits = web.own(&last);
    web.hook(Event::StopActive)
        .assert_silent("the Stop after the cap");

    assert_handed_line(&web.session.inbox(), &last, waits);
    continues_with_a_new_message(&web, CAP + 2, Event::Stop);
}

/// Before each hook run of one turn, api sends web a message: 8 `Stop`s in a row give theirs; then
/// a `PostToolUse`, a tool call, gives its message, and the next `Stop` of the turn gives its own.
#[test]
fn tool_call_between_stops_starts_the_count_again() {
    let web = Web::new(4_913_011_001, "ada-hooks-past-a-tool-call");
    continues_with_a_new_message(&web, 1, Event::Stop);
    for n in 2..=CAP {
        continues_with_a_new_message(&web, n, Event::StopActive);
    }

    continues_with_a_new_message(&web, CAP + 1, Event::PostToolUse);
    continues_with_a_new_message(&web, CAP + 2, Event::StopActive);
}
