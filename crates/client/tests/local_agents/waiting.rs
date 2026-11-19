//! While a device of the account waits for linking, each MCP tool result of a session on a linked
//! device names it once, as its commands do (`link_prompt/main.rs`), also between agents of one
//! device.

use serde_json::json;

use crate::front::Answers;
use crate::lines::{SENT, agent_line_of};
use crate::prompt::{LAPTOP, assert_agent_prompted, waiting_device};
use crate::solo::{API, MESSAGE_FIELD, REPLY_TOOL, SEND_TOOL, Solo, TEXT_FIELD, TO_FIELD, WEB};
use crate::talk::one_message_sent;

/// What api asks web and web answers while Ada's laptop waits; neither is anywhere else.
const ASKED: &str = "local prompt test: api asks web while a laptop waits";
const ANSWERED: &str = "local prompt test: web answers while a laptop waits";

/// Sessions api and web run in `~/work/clientA` on Ada's machine, alone in `clienta`'s groups;
/// then her laptop waits for linking. api's send tool `to` web, web's `inbox`, web's reply tool,
/// api's `inbox`, and api's send tool for a message the front refuses in place of the server each
/// give a result that says once, without an ID, that the server reports, unverified, the `client`
/// device `ada-laptop` waiting and that `skald link` links it.
#[test]
fn each_tool_result_names_the_waiting_device_once() {
    let solo = Solo::new(4_657_011_001, "ada-agents-with-a-laptop");
    let mut api = solo.session(API);
    let mut web = solo.session(WEB);
    let _new = waiting_device(&solo.talk.world, &solo.talk.ada, LAPTOP);
    let mark = solo.mark();

    let sent = api.call(
        SEND_TOOL,
        json!({ TO_FIELD: solo.short(WEB), TEXT_FIELD: ASKED }),
    );

    assert!(!sent.error, "{sent:?}: {}", api.errors());
    assert!(says_sent(&sent.text), "not sent: {sent:?}");
    one_message_sent(&solo.talk.front, solo.default_channel(), mark);
    assert_agent_prompted(&sent.text, LAPTOP);
    let got = web.inbox();
    assert_agent_prompted(&got, LAPTOP);
    let reference = format!("#{}", agent_line_of(&got, ASKED).reference);
    let replied = web.call(
        REPLY_TOOL,
        json!({ MESSAGE_FIELD: reference, TEXT_FIELD: ANSWERED }),
    );
    assert!(!replied.error, "{replied:?}: {}", web.errors());
    assert!(says_sent(&replied.text), "not sent: {replied:?}");
    assert_agent_prompted(&replied.text, LAPTOP);
    let back = api.inbox();
    assert!(back.contains(ANSWERED), "{back}");
    assert_agent_prompted(&back, LAPTOP);
    solo.talk
        .front
        .answer(Answers::Refuse("not-a-member".to_owned()));
    let refused = api.call(
        SEND_TOOL,
        json!({ TO_FIELD: solo.short(WEB), TEXT_FIELD: ASKED }),
    );
    assert!(!says_sent(&refused.text), "{refused:?}");
    assert_agent_prompted(&refused.text, LAPTOP);
}

/// Whether a line of `shown`, a tool's result, is `sent #<ref>`.
fn says_sent(shown: &str) -> bool {
    (shown.lines()).any(|line| line.trim().starts_with(&format!("{SENT} #")))
}
