//! The limit on replies without a human (`docs/protocol/README.md`, "Content";
//! `docs/threat-model.md`, "Prompt injection and loops").
//!
//! - An agent's message carries the `hops` of what it answers: a reply one more than the
//!   original's; a send, from the CLI or the send tool, one more than the highest `hops` of the
//!   messages handed to that agent in its space since the last message the device sent as it, or 0
//!   when none was. A message the device did not send, such as one the limit refuses, does not count
//!   as the last one. The person's messages keep 0.
//! - The device sends no agent message with `hops` over 10: the command or tool sends nothing and
//!   says that the agents answered each other 10 times without a human, and that a person's message
//!   starts the count again. The person's next run warns of it once, with the channel and the
//!   addressee.
//!
//! As the tests of `read` receipts (`read_receipts/main.rs`): the CLI runs on Ada's machine, whose
//! agent claude runs in `~/work/acme`, bound to `acme`, and on her second machine, linked by hand,
//! whose `~/work/acme` is bound to `acme` too; Bob's device is played by the tests with an MLS state
//! of its own. The tests of two agents answering each other take the roles of the story: A is the
//! second machine's agent, B claude on Ada's first machine, and Bob's played device is the test
//! device in the channel that reads each message's `hops`. The tests of how a send counts play the
//! messages handed to claude with Bob's device, as the tests of replies do (`replies/main.rs`), in
//! `acme` or, for another space, in `clienta` and `clientb`. The tests on one device run Ada's
//! machine alone in `clienta`'s groups, with MCP sessions `--agent api` and `--agent web`, as the
//! tests of messages between agents of one device do (`local_agents/main.rs`). MCP sessions are
//! played as in the tests of the MCP server (`mcp_inbox/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - a send or reply the limit refuses: the command fails, and the command's output or the tool's
//!   result has no line `sent #<ref>` and holds the number `10` and the word `person`, as words;
//! - the person's warning of it: a line of the person's `skald recv` that holds the word `warning`,
//!   the channel's ID, as the CLI's other warnings name a channel, and the addressee as `--to`
//!   takes it, `owner/agent`, or in a form that holds it, such as the full address.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod counting;
mod exchanges;
mod loops;
mod one_device;
mod people;
mod spaces;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it; the senders of the
// tests of trust levels; the MCP client of the tests of the MCP server; the second space of the
// tests of the helper; the roles of the tests of `read` receipts;
// what the tests of replies share; Ada's machine alone in its groups, as the tests of messages
// between agents of one device have it.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../replies/replying.rs"]
mod replying;
#[path = "../trust_levels/senders.rs"]
mod senders;
// The tests on one device use none of the tool names `solo` passes on.
#[allow(unused_imports)]
#[path = "../local_agents/solo.rs"]
mod solo;
