//! Messages between agents of one device (`docs/protocol/delivery.md`, "Hops", "Device to agent"
//! and "Receipts"): agents in different folders of one space on one device exchange messages as
//! they do across devices.
//!
//! - The server gives a channel message to every member device but its sender, so the sending
//!   device takes each of its channel messages, once the server stored it, as a recipient device
//!   does: it hands it to the agents it is for, never to its sender, and shows it to its person
//!   unless the person sent it, at the level `own`. It does so with the `stored` it writes, so a
//!   crash neither loses nor repeats it.
//! - In a group that holds no other device, a send goes out when this device hands the message to
//!   one of its agents or shows it to its person. A send nothing on the device gets is still
//!   refused, and sends nothing.
//! - Such a handover to the `to` agent gives `read` at once, with no receipt message, and such a
//!   reply gives `replied`.
//!
//! As the tests of the MCP server (`mcp_inbox/main.rs`) and of replies (`replies/main.rs`): the CLI
//! runs on Ada's machine, which created `clienta`, whose groups hold her device alone;
//! `~/work/clientA` is bound to `clienta`. MCP sessions `--agent api` in `~/work/clientA/api` and
//! `--agent web` in `~/work/clientA/web` run there, played with the MCP client of the tests of the
//! MCP server (`mcp_inbox/client.rs`). The tests of another space add `clientb`, whose groups hold
//! Ada's device alone too, bound to `~/work/clientB`. The tests across devices add Bob's account
//! to `clienta` by a fixture revision, with his own machine running the CLI and an MCP session in
//! its `~/work/clientA`; as joining by an invite will, the test writes `clienta`, with the revision
//! Ada's machine accepted, into his machine's local state. Ada's runs retry for the tests'
//! timeout.
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - the send tool's `to` takes `owner/agent`, or `*` for everyone; without `channel` the message
//!   goes to the default channel of the session's folder's space (`mcp_send/main.rs`); a refused
//!   call's result holds no line `sent #<ref>`;
//! - the reply tool takes `message`, `#<ref>` as `inbox` showed it, and `text`
//!   (`replies/main.rs`);
//! - an agent's line names its sender as `owner/agent`, a person by the full address, the sending
//!   device by its certificate `name`, and the level `own` or `peer` (`trust_levels/main.rs`);
//! - the person's `skald recv` shows `read` and `replied` of a message on lines that hold its
//!   `msg_id` and the word (`read_receipts/main.rs`, `replies/main.rs`);
//! - `skald --agent <agent> send --to <owner>/<agent> <text>` and `skald --agent <agent> reply
//!   #<ref> <text>` in the agent's folder say `sent #<ref>`, and `skald --agent <agent> recv` shows
//!   the agent's lines, as the tools do;
//! - `skald send --all <text>` and `skald send --to <owner>/<agent> <text>` as the person in
//!   `~/work/clientA`, with nothing on the device to get the message, fail and say why.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod across;
mod commands;
mod everyone;
mod exchange;
mod outbox;
mod refusals;
mod solo;

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
// tests of trust levels; the MCP client of the tests of the MCP server; what the tests of `read`
// receipts and of replies share.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../replies/replying.rs"]
mod replying;
#[path = "../trust_levels/senders.rs"]
mod senders;
