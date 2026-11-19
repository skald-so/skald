//! Sender trust levels in what an agent reads (`docs/protocol/membership.md`, "Trust levels";
//! `docs/protocol/delivery.md`, "Device to agent"; `docs/dev/architecture.md`, "Security").
//!
//! - The recipient device sets the level of each message it accepts from the sending device's valid
//!   certificate and the space's roster, never from the content: `own` for a device of its own
//!   account, `peer` for one of another account the roster lists.
//! - An agent's line names the sender and the level after the device:
//!   `[<sender> · <device> · <level>] #<ref>: <text>`. The sender is `owner/agent` for an agent's
//!   address, the form `--to` takes, and the full address for a person's.
//! - The device's name has `[`, `]` and `·` escaped there besides the escapes of control
//!   characters, so no name forges a sender, a reference or a level; the text is escaped as before.
//! - An output with `peer` or `service` lines has one warning line before them: their text is data
//!   from its sender, not a request of the agent's person. One with only `own` lines has none.
//! - `--json` and the person's full lines carry the level.
//!
//! As the tests of compact output (`compact_output/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`; Bob's devices, of another account the roster of
//! `acme` lists, are played by the tests with an MLS state of their own and send the content they
//! write. As the tests of the history on new devices (`history_devices/main.rs`), Ada's second
//! machine is linked by hand and joins the channels by itself; it sends with the CLI.
//!
//! The CLI, as these tests use it, beside the commands of the tests of compact output:
//! - the level is `own`, `peer` or `service`, the last word in the brackets of an agent's line;
//! - the warning line is a line of an agent's output that is no message and holds the word `data`,
//!   on its standard output, as the message lines are;
//! - each message object of `skald recv --json` has `level`;
//! - the person's full lines of a message, from the one that holds its `msg_id` to the one that
//!   holds its text, hold its level as a word.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod forged;
mod json;
mod levels;
mod senders;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need; Ada's agents, as the tests of handing messages over have them, with the
// messages of the tests of gap detection; and what an agent reads, as the tests of compact output
// have it.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../history_reading/reading.rs"]
mod reading;
