//! The hooks give the agent's waiting `own` messages as additional context, each once, as its
//! lines; `Stop` continues the turn with them; the handover gives `read` as `inbox` does.

use crate::hooks::{Event, Web, assert_own_line};
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, json_objects, objects_of};
use crate::receipts::read_lines;
use crate::solo::{API, WEB};
use crate::world::AGENT;

/// What api says to web; neither is anywhere else.
const FIRST: &str = "hook test: api asks web to rebuild the docs";
const SECOND: &str = "hook test: api tells web the docs moved to /guide";

/// web's session runs; api sends web two messages. The person's `skald recv` shows no `read` of
/// either. web's `PostToolUse` hook gives both, each once, as lines from api at `own`; its next run
/// prints nothing, and web's `inbox` shows neither. The person's next `skald recv` shows `read` of
/// each once.
#[test]
fn post_tool_use_gives_each_waiting_own_message_once_and_read_follows() {
    let mut web = Web::new(4_913_001_001, "ada-hooks-a-busy-web");
    let first = web.own(FIRST);
    let second = web.own(SECOND);
    let before = web.solo.person_reads();
    assert_eq!(
        read_lines(&before, first) + read_lines(&before, second),
        0,
        "{before}"
    );

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, FIRST, first, &web.api());
    assert_own_line(&context, SECOND, second, &web.api());
    web.hook(Event::PostToolUse)
        .assert_silent("PostToolUse after the messages went");
    web.assert_inbox_empty("inbox after the hook gave the messages");
    let after = web.solo.person_reads();
    assert_eq!(read_lines(&after, first), 1, "not read once: {after}");
    assert_eq!(read_lines(&after, second), 1, "not read once: {after}");
}

/// What api says to web as the turn ends; it is nowhere else.
const AT_THE_END: &str = "hook test: api asks web for a summary of the turn";

/// web's session runs; api sends web a message. web's `Stop` hook, the first of the turn, gives it
/// as a line from api at `own`, which continues the turn; the `Stop` of the turn it kept going
/// prints nothing, so the turn ends.
#[test]
fn stop_continues_the_turn_with_a_waiting_message_and_then_lets_it_end() {
    let web = Web::new(4_913_002_001, "ada-hooks-the-end-of-a-turn");
    let msg_id = web.own(AT_THE_END);

    let context = web.hook(Event::Stop).context(Event::Stop);

    assert_own_line(&context, AT_THE_END, msg_id, &web.api());
    web.hook(Event::StopActive)
        .assert_silent("Stop of the turn the message kept going");
}

/// What api says to everyone in the channel; it is nowhere else.
const TO_ALL: &str = "hook test: api tells everyone the build is green";

/// web's session runs; api's MCP session sends `*`, everyone in the channel, a message. web's
/// `PostToolUse` hook gives it as a line from api at `own`.
#[test]
fn message_to_everyone_in_the_channel_reaches_the_hook() {
    let web = Web::new(4_913_015_001, "ada-hooks-a-message-to-all");
    let msg_id = web.api_tells_everyone(TO_ALL);

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert_own_line(&context, TO_ALL, msg_id, &web.api());
}

/// What web asks api, and api's reply; neither is anywhere else.
const TO_API: &str = "hook test: web asks api for the changelog";
const REPLY_TEXT: &str = "hook test: api sends web the changelog";

/// api has run once; web's session sends api a message; api takes it with `skald --agent api recv`
/// and replies to it with `skald --agent api reply`. web's `PostToolUse` hook gives the reply; web's `skald recv
/// --json` then shows no status of web's message.
#[test]
fn hook_takes_the_statuses_of_the_agent_s_messages() {
    let mut web = Web::new(4_913_019_001, "ada-hooks-a-reply");
    let api_dir = web.solo.dir(API);
    let api_recv = |web: &Web| {
        (web.machine())
            .start_in(&api_dir, &[AGENT, API, RECV])
            .finish()
    };
    // api's first run makes it an agent of the space, which a message to it needs.
    assert!(api_recv(&web).success);
    let to_api = web.api();
    let asked = web.solo.sends(&mut web.session, &to_api, TO_API);
    let taken = api_recv(&web);
    assert!(taken.success && taken.stdout.contains(TO_API), "{taken:?}");
    let asked_id = asked.to_string();
    let args = [AGENT, API, "reply", asked_id.as_str(), REPLY_TEXT];
    let replied = web.machine().start_in(&api_dir, &args).finish();
    assert!(replied.success, "{replied:?}");

    let context = web.hook(Event::PostToolUse).context(Event::PostToolUse);

    assert!(context.contains(REPLY_TEXT), "no reply: {context}");
    let args = [AGENT, WEB, RECV, JSON_FLAG];
    let after = web.machine().start_in(&web.dir(), &args).finish();
    assert!(after.success, "{after:?}");
    let objects = json_objects(&after.stdout);
    let statuses = objects_of(&objects, asked, "status");
    assert!(
        statuses.is_empty(),
        "statuses after the hook: {statuses:#?}"
    );
}
