//! Replies and `replied` (`docs/protocol/README.md`, "Content"; `docs/protocol/delivery.md`,
//! "Device to agent" and "Receipts"; `docs/protocol/agents.md`, "Person").
//!
//! - `skald reply` and the MCP reply tool take a message, by `#<ref>`, with or without the `#`, or
//!   by its `msg_id`, and a text; no addressee or channel. They send in the original message's
//!   channel, with `to` its `from`, a person's address included, `reply_to` its `msg_id`, and
//!   `hops` 0 from the person, one more than the original's from an agent. An agent's output is
//!   `sent #<ref>`.
//! - The device keeps what a reply needs of each message its person's run showed or it handed to an
//!   agent, for the message's wait: its envelope's `ttl`, at most 7 days after the device wrote it.
//!   As an agent, it replies only to a message handed over to it in its folder's space; any other
//!   is unknown to it. An unknown message, or a reference two kept messages share, sends nothing
//!   and says why, and names nothing of another space.
//! - The sending device takes a reply in the channel of one of its messages, whose `reply_to` is
//!   that message and whose `from` is its `to` agent, as `replied`: the person's `skald recv` shows
//!   it, an agent's only `--json`. A reply to a message without `to`, from another agent, or in
//!   another channel is no `replied`.
//! - A reply's line names the message it answers by its `#<ref>`; `--json` has `reply_to`.
//!
//! As the tests of handing messages over (`agent_handover/main.rs`): the CLI runs on Ada's machine,
//! whose agents claude and codex run in `~/work/acme`, bound to `acme`; Bob's device is played by
//! the tests with an MLS state of its own, writes the content they make, replies included, as one
//! of Bob's agents or as Bob, and reads the content of what Ada's machine sends. The talk's channel
//! is not `acme`'s default one. The tests of another space add `clientb`, as the tests of the MCP
//! server do (`mcp_inbox/spaces.rs`); those of the reply tool play the agent CLI with the MCP
//! client of those tests (`mcp_inbox/client.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above:
//! - `skald reply <message> <text>`, with the global `--agent <name>` as an agent in its folder, and
//!   without it as the person; `<message>` is `#<ref>`, `<ref>` or the `msg_id`. A refusal fails
//!   and says why;
//! - the MCP tool `reply`, with the required arguments `message`, as `skald reply` takes it, and
//!   `text`; it returns what an agent's `skald reply` shows;
//! - the person's `skald recv` shows `replied` of a message on a line that holds its `msg_id` and
//!   the word `replied`, apart from the reply's own lines;
//! - an agent's line of a reply, and the person's, holds a reference of the message it answers, a
//!   `#` and at least 4 leading characters of its reference;
//! - in `skald recv --json`, a reply's message object has `reply_to`, the `msg_id` it answers, and a
//!   `replied` is a status object with `status` `replied`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod refusals;
mod replied;
mod replying;
mod sending;
mod spaces;
mod tools;
mod waiting;

// The helpers of every client test, finding keys in bytes, the machines of the tests of signing in,
// and people in spaces.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../roster_revisions/world.rs"]
mod world;

// The server's tests' person's browser, and the server and the relay of the tests of signing in.
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sign_in/relay.rs"]
mod relay;

// The server's tests' readers of the KeyPackages API's answers, the fixtures and the front of the
// tests of sending, and the channel of the tests of channel messages.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The kit and the device of the tests of channel groups, and the device the tests of receiving play
// over the delivery protocol.
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it; the MCP client and
// the second space of the tests of the MCP server and of the helper.
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
