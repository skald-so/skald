//! The MCP reply tool: it replies as `skald reply` does, by `#<ref>` or by `msg_id`, and returns
//! what an agent's `skald reply` shows.

use serde_json::{Value, json};
use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE};
use crate::client::{MCP, Session};
use crate::lines::{agent_line_of, assert_says_sent};
use crate::replying::{
    BOB_CLAUDE, REPLY, Writes, agent_of, assert_bob_never_reads, assert_reply, bob_reads,
    bob_writes, handles, of_text, the_one,
};
use crate::talk::ADA_HOST;
use crate::world::{ACME, AGENT, World, dir_of};

/// The name the MCP client gives itself at initialization, and the reply tool's arguments.
const CLIENT: &str = "claude-code";
const MESSAGE: &str = "message";
const TEXT: &str = "text";

/// What Bob's device says, and what Ada's claude answers with the tool; none is anywhere else.
const ASKED: [&str; 2] = [
    "reply tool test: bob's claude asks for the api keys rotation date",
    "reply tool test: bob's claude asks who owns the billing service",
];
const ANSWERS: [&str; 2] = [
    "reply tool test: the keys rotate on monday",
    "reply tool test: the payments team owns billing",
];
const NEVER: &str = "reply tool test: a reply to a message the device does not keep";

/// A session of the MCP client [`CLIENT`] as claude in `~/work/acme`, and Bob's device that says
/// two things to Ada's claude in the talk's channel as Bob's claude, with `hops` 1. The session's
/// `inbox` shows both. The reply tool answers the first by `#<ref>`, the second by its `msg_id`:
/// each result is `sent #<ref>` of its reply, which Bob's device reads in the talk's channel from
/// Ada's claude, `to` Bob's claude, with `reply_to` the original and `hops` 2, and in no other
/// channel. A call for a `msg_id` the device does not keep returns no `sent` and sends nothing.
#[test]
fn reply_tool_answers_by_reference_and_by_msg_id_in_the_message_s_channel() {
    let mut agents = Agents::new(
        [4_656_051_001, 4_656_051_002],
        ["ada-replies-over-mcp", "bob-asks-the-mcp-session"],
    );
    let args = [AGENT, CLAUDE, MCP];
    let command = &mut (agents.talk.ada.machine).command_in(&agents.folder, &args);
    let mut session = Session::start(command, CLIENT);
    let (bob, ada) = handles(&agents.talk);
    let (bob_claude, ada_claude) = (
        agent_of(&bob, ACME, BOB_CLAUDE),
        agent_of(&ada, ACME, CLAUDE),
    );
    let at = agents.talk.at();
    let mut asked: Vec<MessageId> = Vec::new();
    for (counter, text) in ASKED.into_iter().enumerate() {
        let writes = Writes::new(&bob_claude, Some(&ada_claude), counter as u64 + 1, text);
        asked.push(bob_writes(&mut agents.talk, at, &writes.hops(1)));
    }
    let shown = session.inbox();
    let first = format!("#{}", agent_line_of(&shown, ASKED[0]).reference);

    let called = [(first, ANSWERS[0]), (asked[1].to_string(), ANSWERS[1])]
        .map(|(message, text)| session.call(REPLY, json!({ MESSAGE: message, TEXT: text })));

    let read = bob_reads(&mut agents.talk, at.1);
    for ((called, text), original) in called.iter().zip(ANSWERS).zip(&asked) {
        assert!(!called.error, "{called:?}");
        let (reply, content) = the_one(&read, text);
        assert_reply(&content, (&ada_claude, &bob_claude), *original, 2);
        assert_says_sent(&called.text, reply);
    }
    let default = agents.talk.space.default;
    let elsewhere = bob_reads(&mut agents.talk, default);
    for text in ANSWERS {
        assert!(
            of_text(&elsewhere, text).is_empty(),
            "{text:?} in {default}"
        );
    }

    let unknown = MessageId::generate().to_string();
    let refused = session.call(REPLY, json!({ MESSAGE: unknown, TEXT: NEVER }));

    let said = refused.text.trim().to_lowercase();
    assert!(!said.is_empty() && !said.starts_with("sent"), "{refused:?}");
    let channels = [agents.talk.channel, default];
    assert_bob_never_reads(&mut agents.talk, &channels, &[NEVER]);
}

/// Ada's machine, signed in, starts a session of [`CLIENT`] in `~/work`. Its `tools/list` lists
/// `reply`, whose input schema requires `message` and `text`.
#[test]
fn tools_list_names_reply_with_its_message_and_text() {
    let world = World::start();
    let ada = world.sign_up(4_656_052_001, "ada-lists-the-reply-tool", ADA_HOST);
    let folder = dir_of(&ada.machine, "work");
    let command = &mut ada.machine.command_in(&folder, &[MCP]);
    let mut session = Session::start(command, CLIENT);

    let listed = session.tools();

    let tools = listed["tools"].as_array().cloned().unwrap_or_default();
    let reply = (tools.iter())
        .find(|tool| tool["name"].as_str() == Some(REPLY))
        .unwrap_or_else(|| panic!("no {REPLY}: {listed}"));
    let schema = &reply["inputSchema"];
    let required: Vec<&str> = (schema["required"].as_array().map(Vec::as_slice))
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for argument in [MESSAGE, TEXT] {
        assert!(
            schema["properties"].get(argument).is_some() && required.contains(&argument),
            "{argument} is not required: {reply}"
        );
    }
}
