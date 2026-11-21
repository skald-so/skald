//! What the hooks put into a session at once stays within Claude Code's hook output limit: a
//! message longer than the limit is cut and ends with how to read it whole; messages that do not
//! fit together wait for the next handover. `inbox` and the agent's `skald recv` take a message's
//! `#<ref>` and return it whole, also one handed over before, while the device keeps it.

use serde_json::json;
use skald_proto::MessageId;

use crate::client::INBOX;
use crate::hooks::{Event, MESSAGE_ARG, Web, assert_cut, assert_own_line};
use crate::kit::RECV;
use crate::lines::{agent_line, agent_lines, assert_handed_line, refers_to};
use crate::solo::WEB;
use crate::world::AGENT;

/// A text of exactly `length` characters that starts with `head` and holds no line break.
fn text_of(head: &str, length: usize) -> String {
    let mut text = format!("{head} ");
    while text.len() < length {
        text.push_str("row of the export, ");
    }
    text.truncate(length - 1);
    text.push('.');
    text
}

/// What starts api's long message; it is nowhere else.
const LONG_HEAD: &str = "limit test: api's export for web, 12000 characters:";

/// api sends web a message of 12,000 characters. web's `PostToolUse` hook gives at most 10,000
/// characters: the message's line, from its start, without its whole text, and as its last line one
/// that names `inbox` and the message's `#<ref>`. web's `inbox` with that `#<ref>` then shows the
/// message's line whole, and so does web's `skald recv` with it; web's `inbox` without one shows
/// nothing of it, and the next `PostToolUse` prints nothing.
#[test]
fn message_over_the_limit_is_cut_and_inbox_returns_it_whole_by_its_reference() {
    let mut web = Web::new(4_913_012_001, "ada-hooks-a-long-export");
    let long = text_of(LONG_HEAD, 12_000);
    let msg_id = web.own(&long);

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    let lines = agent_lines(&context);
    let [line] = lines.as_slice() else {
        panic!("not one message line: {context}");
    };
    assert!(
        refers_to(&line.reference, msg_id) && line.text.starts_with(&long[..1_000]),
        "not the long message's line: {line:?}"
    );
    assert_cut(&context, &long, msg_id);

    let reference = format!("#{}", line.reference);
    let whole = web.session.call(INBOX, json!({ MESSAGE_ARG: reference }));
    assert!(!whole.error, "{whole:?}");
    assert_handed_line(&whole.text, &long, msg_id);
    let args = [AGENT, WEB, RECV, reference.as_str()];
    let recv = web.machine().start_in(&web.dir(), &args).finish();
    assert!(recv.success, "{recv:?}");
    assert_handed_line(&recv.stdout, &long, msg_id);
    let again = web.session.inbox();
    assert!(!again.contains(LONG_HEAD), "inbox shows it again: {again}");
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after the long message went");
}

/// What starts each of api's two messages; neither is anywhere else.
const FIRST_HEAD: &str = "limit test: the first half of api's report";
const SECOND_HEAD: &str = "limit test: the second half of api's report";

/// api sends web two messages of 6,000 characters each, together over the limit. web's first
/// `PostToolUse` hook gives one of them whole and nothing of the other; the second gives the other
/// whole; the third prints nothing.
#[test]
fn messages_that_together_pass_the_limit_go_in_two_handovers() {
    let web = Web::new(4_913_013_001, "ada-hooks-a-report-in-halves");
    let texts = [text_of(FIRST_HEAD, 6_000), text_of(SECOND_HEAD, 6_000)];
    let sent: Vec<MessageId> = texts.iter().map(|text| web.own(text)).collect();

    let first = web.hook(Event::PostToolUse).context(Event::PostToolUse);
    let second = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    let mut given = Vec::new();
    for context in [&first, &second] {
        let lines: Vec<_> = context.lines().filter_map(agent_line).collect();
        let [line] = lines.as_slice() else {
            panic!("not one message line: {context}");
        };
        let at = (texts.iter())
            .position(|text| *text == line.text)
            .unwrap_or_else(|| panic!("not one whole message: {line:?}"));
        assert!(refers_to(&line.reference, sent[at]), "{line:?}");
        given.push(at);
    }
    given.sort_unstable();
    assert_eq!(given, [0, 1], "not each message once");
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after both messages went");
}

/// What api says to web; it is nowhere else.
const SHORT_NOTE: &str = "limit test: api's short note that web reads twice";

/// api sends web a message, which web's `inbox` shows. web's `inbox` with its `#<ref>`, and web's
/// `skald recv` with it, each show the message's line again, from api at `own`.
#[test]
fn inbox_and_recv_return_a_message_handed_over_before_by_its_reference() {
    let mut web = Web::new(4_913_016_001, "ada-rereads-a-note");
    let msg_id = web.own(SHORT_NOTE);
    let line = assert_handed_line(&web.session.inbox(), SHORT_NOTE, msg_id);
    let reference = format!("#{}", line.reference);

    let again = web.session.call(INBOX, json!({ MESSAGE_ARG: reference }));
    let args = [AGENT, WEB, RECV, reference.as_str()];
    let recv = web.machine().start_in(&web.dir(), &args).finish();

    assert!(!again.error, "{again:?}");
    assert_own_line(&again.text, SHORT_NOTE, msg_id, &web.api());
    assert!(recv.success, "{recv:?}");
    assert_own_line(&recv.stdout, SHORT_NOTE, msg_id, &web.api());
}
