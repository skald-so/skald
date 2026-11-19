//! Where a message is: `skald message-status` and the MCP tool `message_status`
//! (`docs/protocol/delivery.md`, "Device to agent" and "Receipts").
//!
//! - The sending device keeps, for each message it sends, the devices it went to: the other member
//!   devices of the channel's group at sending; for each, `stored`, `delivered` or `expired` with
//!   its time; for the `to` agent, `read` and `replied` with their times. On one device, a handover
//!   to the `to` agent counts as `read`.
//! - Of a message it showed or handed over, the device keeps its sending device, the sender's
//!   trust level, its time and the agents it was handed to.
//! - It keeps both for the message's wait: its envelope's `ttl`, at most 7 days.
//! - The command and the tool take a `#<ref>`, with or without `#`, or a `msg_id`, as `skald reply`
//!   does, and give the full `msg_id`, channel, sending device, time, each recipient device by its
//!   name and account with its status and time, and the `to` agent's status. An agent's form is
//!   compact, the person's full, `--json` has every field. An unknown message, or a reference two
//!   kept messages share, gives nothing and says why.
//! - As an agent, only its own messages and those handed to it, in its folder's space.
//!
//! The tests of a message across devices take the roles of the tests of `read` receipts
//! (`read_receipts/main.rs`): A, Ada's second machine, whose agent sends `--to` claude, B's agent on
//! Ada's first machine; C, Bob's device the tests play, is a third device in the channel. The tests
//! of what an agent may ask of take Ada's agents claude and codex in `~/work/acme` with Bob's
//! played device, as the tests of replies do (`replies/main.rs`); those of another space add
//! `clientb`. The test on one device runs Ada's machine alone in `clienta`'s groups, as the tests
//! of messages between agents of one device do (`local_agents/main.rs`). MCP sessions are played
//! with the MCP client of the tests of the MCP server (`mcp_inbox/client.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald message-status <message>`, with the global `--agent <name>` as an agent in its folder,
//!   and without it as the person; `<message>` is `#<ref>`, `<ref>` or the `msg_id`. An unknown
//!   message, or a reference two kept messages share, fails and says why;
//! - the MCP tool `message_status`, with the required argument `message`, as the command takes it;
//!   it returns what the agent's `skald message-status` shows;
//! - the person's form holds the full `msg_id`, the channel's ID, each recipient device's name and
//!   its account's handle, and each status as its word: `stored`, `delivered`, `expired`, `read`,
//!   `replied`; of a message the device got, the sending device's name and the agents it was handed
//!   to;
//! - an agent's form refers to the message by `#<ref>` and holds each recipient device's name and
//!   each status as its word;
//! - `skald message-status --json <message>` prints one JSON object with the fields:
//!   - `msg_id`, the full `msg_id`; `ref`, its reference; `channel`, the channel's ID; `device`, the
//!     sending device's ID; `timestamp`, when it was sent, in Unix seconds;
//!   - of a message the device sent: `recipients`, an array with an object per recipient device,
//!     with `device`, its ID, `device_name`, its certificate's `name`, `account`, its account,
//!     `status`, `stored`, `delivered` or `expired`, and `timestamp`, that status's time; and
//!     `agent`, for a message with `to`, an object whose `read` and `replied` are the times of
//!     those statuses, `null` until they come;
//!   - of a message the device showed or handed over: `level`, the sender's trust level, and
//!     `handed_to`, the full addresses of the agents it was handed to.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod across;
mod expired;
mod levels;
mod received;
mod solo_device;
mod spaces;
mod statuses;
mod unknown;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need; the times of the server's tests.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../../../server/tests/common/times.rs"]
mod times;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it, and what Ada's agents
// say there; the senders of the tests of trust levels; the MCP client of the tests of the MCP
// server; the runs through a helper of the tests of the helper; and what the tests of `read`
// receipts, of replies and of messages between agents of one device share.
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
#[path = "../compact_output/said.rs"]
mod said;
#[path = "../trust_levels/senders.rs"]
mod senders;
// The tool names it re-exports for the tests of messages between agents of one device.
#[allow(unused_imports)]
#[path = "../local_agents/solo.rs"]
mod solo;
