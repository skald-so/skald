//! The lease heartbeat from agent sessions (`docs/protocol/queues.md`, "Claims and leases";
//! `docs/dev/architecture.md`, "Components"; `docs/agent-sessions.md`):
//!
//! - While a session of an agent runs, its `skald mcp` or `skald recv --wait`, the helper extends
//!   each claim of that agent each third of its lease, stating the progress of its last extend;
//!   `skald extend` with another progress extends at once. Without the helper, a one-shot session
//!   extends its agent's claims itself.
//! - A claim as the person, or of an agent with no session running, gets no heartbeat: it ends
//!   after its lease unless `skald extend` keeps it.
//! - The heartbeat stops when the session stops, so the lease ends, and the agent's complete then
//!   gets `claim-lost` and sends no result. A claim the heartbeat finds lost, the agent's next
//!   output says so.
//! - The agent shows `busy` while it holds a claim with `started`, `waiting` with
//!   `awaiting-approval`, and `available` again once it holds none.
//!
//! As the tests of claims (`task_claims/main.rs`): Ada's first machine, whose agent claude
//! enqueues in `~/work/acme`, her second machine, whose agent `codex-v5d8tm` claims in its
//! `~/work/acme`, and Bob's played device, which reads results and presence values. Codex's MCP
//! session runs through a helper, of the second machine's own copy of `skald`, as the tests of the
//! helper run it (`helper/main.rs`), or one-shot. The lease of the channel's queue `default` is
//! shortened to 6 seconds in the server's database, below the shortest a queue may be given. The
//! extends are the requests the relay saw; presence values are read as the tests of presence
//! uploads read them (`presence_uploads/main.rs`), of the second machine's device.
//!
//! The CLI and MCP server, as these tests use them, beside those of the tests of claims and of the
//! MCP server: `skald --agent <name> mcp`, as the session of the agent `<name>`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod scene;

mod one_shot;
mod sessions;
mod statuses;
mod unheld;
mod waits;

// The modules the tests of the channel history share (`history_common/mod.rs`), with the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// The claims scene of the tests of claims, with Ada's agents of the tests of handing messages
// over, the messages of the tests of gap detection, what an agent reads as the tests of compact
// output have it, the senders of the tests of trust levels, the queue commands of the tests of
// queues and the task reads of the tests of task states.
#[path = "../agent_handover/agents.rs"]
mod agents;
// These tests use none of the states `claiming` passes on but some.
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

// The runs of a machine's own copy of `skald` of the tests of the helper, the agent CLI of the
// tests of the MCP server, and the values and presence of the tests of presence uploads.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../helper/helped.rs"]
mod helped;
#[path = "../presence_uploads/presence.rs"]
mod presence;
#[path = "../presence_uploads/values.rs"]
mod values;
