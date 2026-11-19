//! The MCP server says it in its tools' results too.

use serde_json::json;

use crate::agents::{Agents, CLAUDE};
use crate::mcp_client::{MCP, Session};
use crate::prompt::{LAPTOP, assert_agent_prompted, waiting_device};
use crate::world::AGENT;

/// The name the MCP client gives itself at initialization.
const CLIENT: &str = "claude-code";
/// The send tool, and its arguments.
const SEND_TOOL: &str = "send";
const TO_FIELD: &str = "to";
const CHANNEL_FIELD: &str = "channel";
const TEXT_FIELD: &str = "text";
/// Who the send tool's message is to: every agent in the channel.
const EVERYONE: &str = "*";

/// Ada's laptop waits for linking. A session of `skald mcp` as claude in `~/work/acme` on her
/// first machine calls `inbox`: its text says once, without an ID, that the server reports,
/// unverified, the `client` device `ada-laptop` waiting and that `skald link` links it.
#[test]
fn inbox_names_the_waiting_device() {
    let agents = Agents::new(
        [4_178_601_001, 4_178_601_002],
        ["ada-agent-on-mcp-with-a-laptop", "bob-quiet-for-mcp"],
    );
    let talk = &agents.talk;
    let _new = waiting_device(&talk.world, &talk.ada, LAPTOP);
    let command = &mut (talk.ada.machine).command_in(&agents.folder, &[AGENT, CLAUDE, MCP]);
    let mut session = Session::start(command, CLIENT);

    let text = session.inbox();

    assert_agent_prompted(&text, LAPTOP);
}

/// A session of `skald mcp` as claude in `~/work/acme` on Ada's first machine starts; then her
/// laptop waits for linking. The session sends a message with the send tool to `*` in the talk's
/// channel: the result is no error, and its text says once, without an ID, that the server
/// reports, unverified, the `client` device `ada-laptop` waiting and that `skald link` links it.
#[test]
fn send_names_the_waiting_device() {
    let agents = Agents::new(
        [4_178_602_001, 4_178_602_002],
        ["ada-agent-sends-on-mcp-with-a-laptop", "bob-reads-mcp-send"],
    );
    let talk = &agents.talk;
    let command = &mut (talk.ada.machine).command_in(&agents.folder, &[AGENT, CLAUDE, MCP]);
    let mut session = Session::start(command, CLIENT);
    let _new = waiting_device(&talk.world, &talk.ada, LAPTOP);

    let arguments = json!({
        TO_FIELD: EVERYONE,
        CHANNEL_FIELD: talk.channel.to_string(),
        TEXT_FIELD: "link prompt test: sent with the send tool while a laptop waits",
    });
    let called = session.call(SEND_TOOL, arguments);

    assert!(!called.error, "{called:?}: {}", session.errors());
    assert_agent_prompted(&called.text, LAPTOP);
}
