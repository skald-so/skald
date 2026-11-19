//! `read` receipts (`docs/protocol/delivery.md`, "Device to agent", "Receipts" and "Gap
//! detection").
//!
//! - Once a device marks a message whose `to` is an agent handed over to it, by its `skald recv`,
//!   `skald recv --wait` or `inbox`, it issues `read`: one receipt message for the messages of one
//!   sending device and one `from` in a channel that a handover marks, sent with `report` false,
//!   `from` the agent, `to` the messages' `from` and `hops` 0. None for a message without `to`,
//!   and none for the person's run.
//! - The receipt is kept with the mark, so a crash neither loses it nor sends it twice. While the
//!   device's Commit in the channel waits for the server, the receipt waits; the handover does not.
//! - The sending device takes a `read` receipt as the status of a message it lists only from that
//!   message's `to` agent, for a message it sent in the receipt's channel; it ignores any other.
//! - Every device counts a receipt's `counter` in gap detection; devices other than its messages'
//!   sender drop it without a word.
//! - The person's `skald recv` shows `read`; an agent sees it only in `--json`.
//!
//! As the tests of trust levels (`trust_levels/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`, and on her second machine, linked by hand, whose
//! `~/work/acme` is bound to `acme` too; Bob's device is played by the tests with an MLS state of
//! its own. Most tests take the roles of the story: A is the second machine, whose agent sends
//! `--to` claude, B's agent, on Ada's first machine; Bob's played device is C, a third device in
//! the channel, which also reads the receipts B sends. The tests of receipts that B's device does
//! not send itself play B with Bob's device and run A on Ada's first machine (`forged.rs`); the
//! test of a device that is not the messages' sender runs it on Ada's machine with Bob's and
//! Carol's devices played (`others.rs`). MCP sessions are played as in the tests of the MCP server
//! (`mcp_inbox/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald --agent <agent> send --to <owner>/<agent> --channel <channel> <text>` and
//!   `skald --agent <agent> send --all --channel <channel> <text>` (`send_addressee/main.rs`);
//! - the person's `skald recv` shows a `read` status in a line that holds the message's `msg_id`
//!   and the word `read`, as it shows other statuses;
//! - `skald recv --json`, as an agent too, gives a `read` status as an object with the message's
//!   `msg_id`, `status` `read`, `channel` and `device`, the ID of the device that issued it, as it
//!   gives other statuses;
//! - an agent's short lines hold no word `read`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod addressees;
mod batches;
mod commits;
mod crashes;
mod forged;
mod handovers;
mod json;
mod others;
mod receipts;
mod sessions;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need; Ada's agents, as the tests of handing messages over have them, with the
// messages of the tests of gap detection; what an agent reads, as the tests of compact output have
// it; the senders of the tests of trust levels; the MCP client of the tests of the MCP server; and
// the runs through a helper of the tests of the helper and of `skald recv --wait`.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../trust_levels/senders.rs"]
mod senders;
#[path = "../recv_wait/waits.rs"]
mod waits;
