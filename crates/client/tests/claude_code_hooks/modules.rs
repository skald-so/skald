// The shared modules of the tests of the Claude Code paths, which `include!` puts at the root of the
// hook handler's tests and of the end-to-end tests: Ada's machine alone in `clienta`, as the tests of
// messages between agents of one device have it; the senders of the tests of trust levels; what the
// tests of `read` receipts share; the MCP client of the tests of the MCP server; what an agent
// reads, as the tests of compact output have it; Ada's agents, as the tests of handing messages over
// have them, with the messages of the tests of gap detection; the second machines of the tests of
// the history on new devices and the helpers of the tests of reading the history they need; and the
// modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../local_agents/solo.rs"]
mod solo;
#[path = "../trust_levels/senders.rs"]
mod senders;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
