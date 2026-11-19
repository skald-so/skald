//! Inbound modes, the sender allowlist and `refused` (`docs/protocol/delivery.md`, "Device to
//! agent" and "Receipts").
//!
//! - Each agent of the device in a space has an inbound mode, `accept` by default, `hold` or
//!   `refuse`, and an allowlist of senders it takes messages from whatever the mode: an agent's
//!   address in the forms `--to` takes, or a member's handle for all of that member's agents and
//!   the person. The person sets and lists them; as an agent, the command changes nothing and says
//!   why. They are kept in the local state.
//! - When the device takes in a message for an agent, the mode decides: a sender on the allowlist,
//!   or `accept`, keeps it for the agent; `refuse` hands it to no agent and, for a message `to` the
//!   agent, issues `refused` with a reason for its sender. A message taken in keeps that decision
//!   when the mode changes.
//! - `refused` goes as `read` does: one receipt message for the messages of one sending device,
//!   `from` and reason in a channel, `from` the agent, `to` the messages' `from`, `hops` 0, sent
//!   with `report` false, kept with the decision, so a crash neither loses it nor sends it twice. On
//!   one device, the status is set with no receipt message.
//! - The sending device takes `refused` only from the message's `to` agent, for its message in that
//!   channel. Its person's `skald recv` shows it with the reason, its agent's lines as a problem of
//!   the agent's own message, `--json` in full. The refusing device's person's run shows the
//!   message marked as refused for that agent.
//!
//! As the tests of `read` receipts (`read_receipts/main.rs`): A is Ada's second machine, whose agent
//! sends `--to` claude, B's agent, on Ada's first machine, whose agents run in `~/work/acme`, bound
//! to `acme`; C is Bob's played device in the channel, which reads the receipts B sends. The test of
//! receipts B's device does not send itself plays B with Bob's device (`forged.rs`), and the test on
//! one device runs Ada's machine alone in `clienta`'s groups, as the tests of agents of one device
//! do (`local_agents/solo.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald set-inbound-mode <agent> <mode>`, `<mode>` one of `accept`, `hold` and `refuse`, sets
//!   the mode of the device's agent `<agent>` in the working directory's space;
//! - `skald allow-sender <agent> <sender>` puts `<sender>`, an agent's address as `--to` takes it
//!   or a member's handle, on that agent's allowlist;
//! - `skald inbound-modes` lists each agent of the device in the working directory's space with its
//!   mode, on a line that holds the agent's name and the mode, and the senders on its allowlist;
//! - with the global `--agent <name>`, `skald set-inbound-mode` and `skald allow-sender` fail, say
//!   why and change nothing;
//! - the sending device's person's `skald recv` shows `refused` on a line that holds the message's
//!   `msg_id`, the word `refused` and the reason; its agent's `skald recv` on one line that holds
//!   the message's `#<ref>`, the word `refused` and the reason, and is no message;
//!   `skald recv --json`, as an agent too, gives it as an object with the message's `msg_id`,
//!   `status` `refused`, `channel`, `device`, the ID of the device that issued it, and the reason;
//! - the refusing device's person's `skald recv` shows the message, and a line that holds its
//!   `msg_id`, the word `refused` and the agent's name.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod allowlist;
mod changes;
mod commands;
mod crashes;
mod everyone;
mod forged;
mod inbound;
mod json;
mod one_device;
mod refusing;

// The roles of the tests of `read` receipts, with the senders of the tests of trust levels and Ada's
// agents, as the tests of handing messages over have them.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../trust_levels/senders.rs"]
mod senders;

// Ada's machine alone in its groups, of the tests of agents of one device, with the MCP client of
// the tests of the MCP server. `solo` re-exports the client's names its own tests use.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[allow(unused_imports)]
#[path = "../local_agents/solo.rs"]
mod solo;

// What an agent reads, as the tests of compact output have it; the waits of the tests of
// `skald recv --wait`, with the runs through a helper of the tests of the helper.
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../recv_wait/waits.rs"]
mod waits;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices, the helpers of the tests of reading the
// history, and the messages of the tests of gap detection, which the modules above need.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../history_reading/reading.rs"]
mod reading;
