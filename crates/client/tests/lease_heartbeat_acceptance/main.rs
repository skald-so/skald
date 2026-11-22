//! The lease heartbeat from agent sessions (`docs/protocol/queues.md`, "Claims and leases";
//! `docs/dev/architecture.md`, "Components"; `docs/agent-sessions.md`, "The MCP server").
//!
//! - While a session of an agent runs, its `skald mcp` or `skald recv --wait`, the device extends
//!   each claim of that agent each third of the claim's lease, with the progress of the claim's
//!   last extend: through the helper, or, without one, from the one-shot session itself. A claim's
//!   session is any session of its agent on the device.
//! - A claim as the person, or of an agent with no session running, gets no heartbeat: it holds
//!   only as long as `skald extend` keeps it.
//! - The heartbeat stops when the claim ends or is lost and when the session stops, so the lease
//!   ends; the stopped agent's `skald complete` then says the claim is lost, and no result goes
//!   out. A claim found lost is said in the session's next tool result.
//! - The agent's presence: `busy` while it holds a claim with `started`, `waiting` with
//!   `awaiting-approval`, `available` again once it holds none.
//!
//! As the tests of claiming tasks (`task_claims/main.rs`): the CLI runs on Ada's machine, where
//! claude enqueues, and on her second machine, linked by hand, where the agent `codex-v5d8tm`
//! claims in `~/work/acme`; Bob's device, of another account, is played by the tests and reads the
//! results and presence values sent in the channel. The runs that may go through a helper run the
//! second machine's own copy of `skald` (`helper/helped.rs`); the agent CLI is the MCP client the
//! tests of the MCP server play (`mcp_inbox/client.rs`).
//!
//! The lease is shortened for the tests: the server's copy of the queue `default` gets a lease of
//! [`scene::LEASE`] seconds, below the range a queue's settings take, so the claims it hands out
//! carry it. Each heartbeat is seen as a request the second machine sends through the relay; the
//! server's reads show the task held by the same claim, or no longer held.
//!
//! The CLI, as these tests use it, beside the commands of the tests of claiming tasks:
//! - `skald --agent <name> mcp`, through the helper or with `SKALD_ONE_SHOT=1`, and
//!   `skald --agent <name> recv --wait` through the helper, each a session of that agent;
//! - the MCP tool `inbox`, whose result says `lost` and names the task, by its `#<ref>` or ID, once
//!   the device found a claim of the session's agent lost.
//!
//! The server's interface, as these tests use it: `GET .../queues/{queue}/tasks/{task}`,
//! `POST .../queues/{queue}/tasks/{task}/extend` as the machine sends it, and
//! `POST .../tasks/{task}/cancel`, as the tests of claiming tasks use them;
//! `GET /spaces/{space}/channels/{channel}/presence`, as the tests of presence uploads use it.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod scene;

mod ended;
mod lost;
mod one_shot;
mod sessions;
mod statuses;
mod unheld;

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

// The history entries of the tests of history entries; the browsers of the tests of history wraps.
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../history_wraps/wraps.rs"]
mod wraps;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it; the senders of the
// tests of trust levels; the queue commands and requests of the tests of queues; the task states
// and reads of the tests of task states; the claims of the tests of claiming tasks.
#[path = "../agent_handover/agents.rs"]
mod agents;
// Its states beside the claim commands, some of which these tests do not name.
#[allow(unused_imports)]
#[path = "../task_claims/claiming.rs"]
mod claiming;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../queues/queuing.rs"]
mod queuing;
#[path = "../trust_levels/senders.rs"]
mod senders;
#[path = "../task_states/states.rs"]
mod states;

// The runs of a machine's own copy of `skald`, as the tests of the helper have them; the agent CLI
// of the tests of the MCP server; the presence values and a device's proof of the tests of
// presence uploads.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../presence_uploads/presence.rs"]
mod presence;
#[path = "../presence_uploads/values.rs"]
mod values;
