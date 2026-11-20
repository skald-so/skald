//! The presence directory: `skald agents` and the MCP tool `agents` list the agents of a space from
//! the presence values of its channels (`docs/protocol/agents.md`, "Presence").
//!
//! - The device gets each channel's values and opens each with the epoch's history key it holds.
//!   It drops, warning its person, each that fails a reader check: of a device without a valid
//!   certificate or not admitted to the channel, with a `revision` not above the highest it
//!   accepted for that channel and device, given in another channel's values, or listing an agent
//!   that is no `from` that device may send as. A value that fails only check 4 shows its agents
//!   `offline`, last seen before its `expires`.
//! - The command and the tool list the agents of the working directory's space, or of a space the
//!   person names, each once per device, with its state, since when, note, runtime, device and
//!   trust level: the person's lines in full, an agent's compact, `--json` with every field. Notes
//!   from `peer` and `service` devices follow the warning line of the tests of trust levels,
//!   escaped as device names are there, so no note forges a line.
//! - An address that two devices of one account list, neither as `offline`, is marked. A device
//!   that registers an agent whose address another device of its account lists as running warns
//!   its person and the agent's next result, once.
//! - As an agent, the device names nothing of other spaces.
//! - The values a client uploads list an MCP session's agent `available` while it runs through the
//!   helper and `offline` once it ended (`uploaded.rs`).
//!
//! As the tests of trust levels (`trust_levels/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`; Bob's devices, of another account the roster of
//! `acme` lists, are played by the tests with an MLS state of their own. A second machine of
//! Ada's, and one of Bob's, are linked by hand, as in the tests of the history on new devices
//! (`history_devices/main.rs`), and played too. The played devices upload presence values sealed
//! with their keys, and the tests seal values as a device with its signing key and the epoch's
//! history key a played device holds. A proxy changes the server's answers with the relay of the
//! tests of signing in. MCP sessions are played with the MCP client of the tests of the MCP server
//! (`mcp_inbox/client.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald agents [<space>]` lists the agents of the working directory's space, or of `<space>`,
//!   as the person; with the global `--agent <name>`, as that agent in its folder;
//! - the person's form has one line per agent and device that holds the agent's full address, its
//!   state as its word, `available`, `busy`, `waiting` or `offline`, its note, its runtime, its
//!   device's certificate `name` and its trust level as a word;
//! - an agent's form has one line per agent and device,
//!   `[<owner>/<agent> · <device> · <level>] <rest>`, with the device's certificate `name`, its
//!   trust level, and the state as its word, the runtime and the note in `<rest>`; the note
//!   escaped so that it holds no line break, `[`, `]` or `·`; the warning line of the tests of trust
//!   levels, which holds the word `data`, before the first line at a level other than `own`; no
//!   UUID;
//! - `skald agents --json` prints one JSON object per line, per agent and device, with `agent`, its
//!   full address; `state`; `since`, in Unix seconds; `note` and `runtime`, or `null`; `device`,
//!   the device's ID; `device_name`; `level`; `duplicate`, `true` when another device of the
//!   account lists the address and neither as `offline`; `last_seen`, for a value past its
//!   `expires`, that `expires` in Unix seconds, otherwise `null`;
//! - the MCP tool `agents`, without arguments, returns the lines the agent's `skald agents` shows;
//! - a dropped value: a warning line of the person's run that holds the word `presence` and names
//!   the value's device by its certificate `name` or ID;
//! - an agent registered on a device while another device of its account lists its address as
//!   running: one line holding `<owner>/<agent>` and the other device's certificate `name`, in the
//!   agent's next tool result and as a warning in the person's next run; in neither later.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod dropped;
mod duplicates;
mod listing;
mod notes;
mod spaces;
mod uploaded;
mod values;

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
// tests of trust levels; the MCP client of the tests of the MCP server; and the second space of
// the tests of the helper.
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
#[path = "../trust_levels/senders.rs"]
mod senders;
