//! Presence uploads from the client (`docs/protocol/agents.md`, "Presence", "Uploads";
//! `docs/dev/architecture.md`, "Components"):
//!
//! - A device's value in a channel lists every agent it registered in the channel's space:
//!   `available` while a session of it runs, `offline` otherwise, `since` when it entered that
//!   state, `runtime` the MCP client's name made a name; `serves` stays empty. A device with no
//!   agent uploads empty `agents`.
//! - A session runs while its `skald mcp` or `skald recv --wait` runs. The MCP server holds a
//!   session of the helper for its whole run, so the helper does not exit idle under it.
//! - An agent sets its state, `busy` or `available`, and its note from the CLI and an MCP tool; a
//!   note over 128 characters is refused, naming the limit.
//! - The helper uploads in every channel whose group the device is a member of, all together, when
//!   an agent's state or note changes and before `expires`, 900 seconds ahead, passes; in a channel
//!   on entering a new epoch there; and its agents as `offline` before it exits. A one-shot run
//!   with an agent uploads only when its value changed or the last one is near `expires`.
//! - `revision` rises by one per value and is kept as durably as the counters, so a killed run
//!   never reuses one. A value is sealed only under the current epoch judged clean.
//!
//! As the tests of the MCP server (`mcp_inbox/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`; Bob's device is played by the tests with an MLS
//! state of its own and is in each of Ada's channels. The tests read Ada's values as Bob's device:
//! they get each channel's values from the server, proving his device, and open Ada's with his
//! history key of the value's epoch and the certificate of Ada's device (`values.rs`). The runs
//! that may go through a helper run the machine's own copy of `skald` (`helper/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald --agent <name> set-status <busy|available> [--note <text>]` sets the agent's state and
//!   note in one operation; a note over 128 characters fails the run, saying `128`;
//! - the MCP tool `set_status`, with the arguments `state`, `busy` or `available`, and `note`, does
//!   the same for its session's agent; a note over 128 characters gives an error result saying
//!   `128`;
//! - `SKALD_PRESENCE_EXPIRES=<seconds>` sets how far after an upload its value's `expires` is, 900
//!   when unset; a helper takes it from the command that starts it.
//!
//! The server's interface, as these tests use it: `GET /spaces/{space}/channels/{channel}/presence`
//! with `Skald-Device-Proof`, `200` with `{"values": [<bytes>]}`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod epochs;
mod one_shot;
mod refresh;
mod revisions;
mod sessions;
mod statuses;
mod values;

// The modules the tests of the channel history share (`history_common/mod.rs`), and the unclean
// epoch of the tests of history entries.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_entries/entries.rs"]
mod entries;

// Ada's agents, as the tests of handing messages over have them; the messages of the tests of gap
// detection; the runs of a machine's own copy of `skald`, as the tests of the helper have them; and
// the agent CLI of the tests of the MCP server.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../helper/helped.rs"]
mod helped;
