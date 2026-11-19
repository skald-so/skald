//! The MCP server says it in its tools' results too.

use crate::agents::{Agents, CLAUDE};
use crate::mcp_client::{MCP, Session};
use crate::prompt::{LAPTOP, assert_agent_prompted, waiting_device};
use crate::world::AGENT;

/// The name the MCP client gives itself at initialization.
const CLIENT: &str = "claude-code";

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
