//! Own text written to the socket counts as handed over, and `read` goes, only once the session
//! file confirms it: an entry that went to the model holds its `msg_id`, and the model's own entry
//! follows, a reply, a tool call or reasoning, not an API error. A tool result, a queued prompt or
//! a hook's own output confirms nothing. Meanwhile no other path hands the message over.

use crate::hooks::Event;
use crate::lines::agent_lines;
use crate::receipts::shows_read_once;
use crate::registered::{Ada, QUIET, no_read_for};
use crate::solo::WEB;
use crate::standin::{Standin, Take};

/// What api says to web; it is nowhere else.
const QUEUED_SAYS: &str = "confirm test: api asks web to bump the version";

/// web's session starts; it queues what comes, as a busy session does. api sends web a message;
/// the socket gets it. Over [`QUIET`], the person's `skald recv` shows no `read` of it. web's
/// `PostToolUse` and `Stop` hooks print nothing, and web's `inbox` shows no message. The session
/// takes the message into the conversation: still no `read` over [`QUIET`]. The model replies:
/// the person's `skald recv` shows `read` of it once.
#[test]
fn read_follows_once_the_model_s_reply_follows_the_message_in_the_session_file() {
    let mut ada = Ada::new(4_919_411_001, "ada-confirms-by-reply");
    let session = Standin::new(Take::Queue);
    ada.starts(WEB, &session);

    let msg_id = ada.web.own(QUEUED_SAYS);
    session.wait_lines(1);

    no_read_for(ada.machine(), msg_id, QUIET, "queued");
    (ada.hook(WEB, &session, Event::PostToolUse))
        .assert_silent("PostToolUse while the socket has the message");
    (ada.hook(WEB, &session, Event::Stop)).assert_silent("Stop while the socket has the message");
    let shown = ada.web.session.inbox();
    assert!(agent_lines(&shown).is_empty(), "inbox: {shown}");
    session.dequeue();
    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "in the conversation, no reply",
    );
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);
}

/// What api says to web; neither is anywhere else.
const BEFORE_REASONING: &str = "confirm test: api asks web to think it over";
const BEFORE_TOOL_CALL: &str = "confirm test: api asks web to run the linter";

/// web's session starts and takes what comes into the conversation. api sends web a message; once
/// the socket has it, the model reasons: the person's `skald recv` shows `read` of it once. api
/// sends web another; once the socket has it, the model calls a tool: `read` of it once.
#[test]
fn reasoning_or_a_tool_call_of_the_model_confirms_the_message_too() {
    let ada = Ada::new(4_919_412_001, "ada-confirms-by-reasoning");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);

    let first = ada.web.own(BEFORE_REASONING);
    session.wait_lines(1);
    session.model_reasons();
    shows_read_once(ada.machine(), first);

    let second = ada.web.own(BEFORE_TOOL_CALL);
    session.wait_lines(2);
    session.model_calls_a_tool();
    shows_read_once(ada.machine(), second);
}

/// What api says to web; it is nowhere else.
const BEFORE_ERROR: &str = "confirm test: api asks web while the model is down";

/// web's session starts and takes what comes into the conversation. api sends web a message; once
/// the socket has it, the call to the model fails with an API error: no `read` over [`QUIET`].
/// Then the model replies: `read` of it once.
#[test]
fn an_api_error_after_the_message_confirms_nothing() {
    let ada = Ada::new(4_919_413_001, "ada-confirms-not-on-errors");
    let session = Standin::new(Take::Accept);
    ada.starts(WEB, &session);

    let msg_id = ada.web.own(BEFORE_ERROR);
    session.wait_lines(1);
    session.api_error();

    no_read_for(ada.machine(), msg_id, QUIET, "after an API error");
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);
}

/// What api says to web; it is nowhere else.
const ELSEWHERE_SAYS: &str = "confirm test: api asks web, its line read elsewhere";

/// web's session starts; it queues what comes. api sends web a message; once the socket has it, a
/// tool result and a hook's own output, each holding the message's `msg_id` and the socket's text
/// of it, go into the session file, and the model replies: the queued prompt, the tool result and
/// the hook's output confirm nothing, so no `read` over [`QUIET`]. The session takes the message
/// into the conversation and the model replies: `read` of it once.
#[test]
fn a_tool_result_a_queued_prompt_or_a_hook_s_output_confirms_nothing() {
    let ada = Ada::new(4_919_414_001, "ada-confirms-not-elsewhere");
    let session = Standin::new(Take::Queue);
    ada.starts(WEB, &session);

    let msg_id = ada.web.own(ELSEWHERE_SAYS);
    let line = session.wait_lines(1).remove(0);
    let seen = format!("{msg_id}\n{}", line.content());
    session.tool_result(&seen);
    session.hook_output(&seen);
    session.model_replies();

    no_read_for(
        ada.machine(),
        msg_id,
        QUIET,
        "seen outside the conversation",
    );
    session.dequeue();
    session.model_replies();
    shows_read_once(ada.machine(), msg_id);
}
