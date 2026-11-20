//! `hold` until the person decides (`docs/protocol/delivery.md`, "Device to agent" and
//! "Receipts"):
//!
//! - In `hold`, a message from a sender off the agent's allowlist is kept for the person, not for
//!   the agent; one `to` the agent gets `held`, issued as `refused` is: one receipt message, `from`
//!   the agent, `to` the message's `from`, `hops` 0. On one device, the status is set with no
//!   receipt message.
//! - The person's `skald recv` says how many messages wait for a decision. A command lists them,
//!   each with its agent, channel, sender with device and trust level, time, `#<ref>` and text, and
//!   accepts or refuses each, refusing with an optional reason. It runs only as the person at a
//!   terminal; as an agent or without a terminal it decides nothing and says why.
//! - Accepted, the message is kept for the agent and handed over as any other, then gets `read`;
//!   refused, it gets `refused` with the reason. One whose wait ends undecided gets `refused` with
//!   a reason saying no one decided. A decision and its receipt are kept together, so a crash
//!   neither loses nor repeats them.
//! - The sending device takes `held` only from the `to` agent. Its person's run shows `held`, then
//!   `read` or `refused`; its agent's lines show `held` and `refused` once each.
//!
//! As the tests of inbound modes (`inbound_modes/main.rs`): A is Ada's second machine, whose agent
//! sends `--to` claude, B's agent, on Ada's first machine, whose agents run in `~/work/acme`; C is
//! Bob's played device in the channel, which reads the receipts B sends and plays a sender whose
//! message's wait is short. The test on one device runs Ada's machine alone in `clienta`'s groups.
//! The person's decisions are typed at a pseudo-terminal, or a pseudo-console on Windows.
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald set-inbound-mode <agent> hold` sets the agent's mode to `hold`;
//! - `skald held`, run as the person at a terminal in a folder of the space, takes the messages
//!   that wait for a decision one at a time: it shows the message, with its agent's name, the
//!   channel's ID, the sender's address, the sending device's name and trust level, its time, its
//!   `#<ref>` and its text, then reads one line, `accept` or `refuse`; after `refuse`, the next line
//!   is the reason, an empty one giving none. It sends the `refused` of a refusal itself. Once each
//!   is decided it exits successfully; with none waiting it shows none and exits successfully. As
//!   an agent (`--agent`), or with its input or output not a terminal, it fails, says why and
//!   decides nothing;
//! - while messages wait for a decision, the person's `skald recv` prints one line that holds
//!   their number as a word and `skald held`; with none waiting, no line holds `skald held`;
//! - the sending device's person's `skald recv` shows `held` on a line that holds the message's
//!   `msg_id` and the word `held`; its agent's `skald recv` on one line that holds the message's
//!   `#<ref>` and the word `held`, and is no message; `read` and `refused` as their tests have them;
//! - the reason of a `refused` whose wait ended undecided holds `no one decided`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod allowlist;
mod crashes;
mod deciding;
mod everyone;
mod expiry;
mod forged;
mod held;
mod holding;
mod one_device;

// The shared modules, by name: Ada's agents, as the tests of handing messages over have them; the
// MCP client of the tests of the MCP server; the second machines of the tests of the history on
// new devices; the messages of the tests of gap detection; the runs through a helper of the tests
// of the helper; the modules the tests of the channel history share (`history_common/mod.rs`); the
// commands and what is shown of `refused` in the tests of inbound modes; what an agent reads, as
// the tests of compact output have it; the helpers of the tests of reading the history; the roles
// of the tests of `read` receipts, with the senders of the tests of trust levels; Ada's machine
// alone in its groups, of the tests of agents of one device, which re-exports the client's names
// its own tests use; the person's terminal, as the tests of the recovery key have it; and the
// waits of the tests of `skald recv --wait`.
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
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../inbound_modes/inbound.rs"]
mod inbound;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../trust_levels/senders.rs"]
mod senders;
#[allow(unused_imports)]
#[path = "../local_agents/solo.rs"]
mod solo;
#[path = "../recovery_key/terminal.rs"]
mod terminal;
#[path = "../recv_wait/waits.rs"]
mod waits;
