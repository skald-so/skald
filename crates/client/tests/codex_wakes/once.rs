//! A Codex session has one queued signal at a time: messages that come before the turn it starts
//! runs add no `codex queue`; after that turn, the next message gives another.

use skald_proto::MessageId;

use crate::codex::Codex;
use crate::queued::assert_inbox_gives;
use crate::standin::{assert_queued, assert_quiet, web_in_app};

/// How many messages api sends web while the first one's signal waits in the queue.
const WHILE_QUEUED: usize = 10;

/// What api says to web: first, while its signal waits, and after the turn it started.
const FIRST: &str = "codex wake test: api asks web for the release notes";
const AFTER: &str = "codex wake test: api asks web, after the woken turn, for the tag";

/// The text of the message `at` that api sends web while the first one's signal waits.
fn while_queued(at: usize) -> String {
    format!("codex wake test: note {at:02} of {WHILE_QUEUED} while a signal is queued")
}

/// web's session starts in a Codex app of its own: it is idle. api sends web a message: the app's
/// `codex` runs `codex queue` once. Before a turn runs, api sends web ten more: the app's `codex`
/// does not run again. The turn the signal starts runs and ends. api sends web another message:
/// the app's `codex` runs `codex queue` once more, for web's thread, and then not again. web's
/// `inbox` gives all twelve messages.
pub fn messages_while_a_queued_signal_waits_add_no_codex_queue() {
    let mut codex = Codex::new(4_928_003_001, "ada-codex-wake-once");
    let (app, web, mut process) = web_in_app(&codex);

    let mut texts = vec![FIRST.to_owned()];
    let mut msg_ids: Vec<MessageId> = vec![codex.web.own(FIRST)];
    app.await_runs(1);
    for at in 1..=WHILE_QUEUED {
        let text = while_queued(at);
        msg_ids.push(codex.web.own(&text));
        texts.push(text);
    }
    assert_quiet(&[(&app, 1, "codex queue while a signal waits in the queue")]);

    process.turn(&codex, &web);
    msg_ids.push(codex.web.own(AFTER));
    texts.push(AFTER.to_owned());
    let ran = app.await_runs(2);
    let all: Vec<&str> = texts.iter().map(String::as_str).collect();
    for each in &ran {
        assert_queued(each, (&web, &app), &all, &msg_ids, &[]);
    }
    assert_quiet(&[(&app, 2, "one more codex queue, after the woken turn")]);

    let sent: Vec<(&str, MessageId)> = all.iter().copied().zip(msg_ids.iter().copied()).collect();
    assert_inbox_gives(&mut codex, &sent);
}
