//! A channel's queues and enqueued tasks in the CLI (`docs/protocol/queues.md`, "Queues", "Tasks"
//! and "Queue key"; `docs/dev/architecture.md`, "Queue").
//!
//! - The person lists a channel's queues with their settings, creates, changes and deletes them;
//!   an agent lists them and is refused the rest, which changes nothing. The server's refusal is
//!   named, such as `not-allowed` for a member who is neither the queue's creator nor an admin.
//! - `skald enqueue` uploads one task, sealed to the `pk` of the channel's current epoch, an epoch
//!   the device judged clean, and signed by the device: content `from` the agent or the person,
//!   with the `task` body, no `counter`, `hops` as a send has them, and the request's `messageId`
//!   the task's ID. Under an epoch the device judged unclean, nothing is enqueued, saying why.
//! - The task is stored once per `idempotency_key`: a second task with the key is refused, naming
//!   the first's `#<ref>`. The local state keeps a task before its upload, so a retry, after a
//!   crash too, carries the same bytes.
//! - `--to` names the addressee: the agent in `to`, and in `sched` the one device whose valid
//!   presence value lists it; with none or several, nothing is enqueued, saying why.
//! - With no `deadline` from its user, the device sets one 7 days after enqueue if a device of
//!   another account may claim the task; one the user sets is kept.
//!
//! As the tests of trust levels (`trust_levels/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`, and, where a test needs it, on her second
//! machine, linked by hand, whose `~/work/acme` is bound to `acme` too; Bob's device, of another
//! account the roster of `acme` lists, a member who is no admin, is played by the tests with an
//! MLS state of its own. The tests open each task the server holds with the queue key of Bob's
//! history key of the task's epoch, and check its signature with the certificate of the sending
//! device. The test of a member who is no admin runs the CLI on Bob's machine, which created
//! `acme` and handed its admin role to Ada, as the tests of the history period do
//! (`history_rotation/period.rs`); Ada's device is played there. The test of an unclean epoch
//! makes one as the tests of history entries do (`history_entries/unclean.rs`). The tests of
//! `hops` play the messages handed to claude with Bob's device, as the tests of the limit on
//! replies without a human do (`hop_limit/main.rs`).
//!
//! The CLI, as these tests use it, beside the commands of the tests above. `<channel>` is a
//! channel's ID, as `skald send --channel` takes it; without `--channel`, a command takes the
//! default channel of the working directory's space, as `skald send` does.
//! - `skald queues [--channel <channel>]`: one line per queue of the channel holding its name, its
//!   mode, `at-least-once` or `at-most-once`, its lease in seconds and its `max_attempts`, each as
//!   a word; as an agent too.
//! - `skald create-queue [--channel <channel>] [--mode <mode>] [--lease <seconds>]
//!   [--max-attempts <n>] <name>`, `skald change-queue [--channel <channel>] [--lease <seconds>]
//!   [--max-attempts <n>] <name>` and `skald delete-queue [--channel <channel>] <name>`: the
//!   settings left out keep their value, or the server's default at creation. As an agent, each
//!   fails and makes no request that changes a queue. A refusal from the server fails the run,
//!   which names its problem as a word, such as `not-allowed`.
//! - `skald enqueue [--channel <channel>] [--queue <name>] [--to <address>] [--idempotency-key
//!   <key>] [--deadline <time>] <text>`: `--queue` is `default` when left out; `--to` takes an
//!   agent's address as `skald send --to` does; the key's UTF-8 bytes are the `idempotency_key`;
//!   `<time>` is an RFC 3339 time, as `skald create-service-token --expires` takes one. The
//!   request is an A2A Message with the role `ROLE_USER` and one text part, `<text>`. The output
//!   holds the task's reference, `#<ref>`, made from its ID as a message's is from its `msg_id`.
//! - A refused `skald enqueue` fails and uploads no task: one with an `idempotency_key` another
//!   task of the queue has holds the word `duplicate` and the `#<ref>` of that task; one under an
//!   epoch the device judged unclean holds the word `epoch`; one naming a queue the channel lacks
//!   names it; one whose addressee no valid presence value lists names the agent, and one that two
//!   devices list names both devices by their certificate `name`; one over the limit on `hops`
//!   holds the number `10` and the word `person`, as a send the limit refuses does.
//! - A task whose upload the server has not answered waits in the local state; the device's next
//!   `skald enqueue` or `skald recv` uploads it first, with the same bytes.
//!
//! The server's interface, as these tests use it, each request proving its device as
//! `/mailbox/send` does: `GET` and `POST /spaces/{space}/channels/{channel}/queues`, and `GET
//! /spaces/{space}/channels/{channel}/queues/{queue}/tasks`, as the server's tests of queues
//! (`crates/server/tests/queues/main.rs`) use them; the machines upload a task by `POST` to the
//! latter.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod addressees;
mod claimers;
mod deadlines;
mod forms;
mod hops;
mod idempotency;
mod limit;
mod management;
mod queuing;
mod retries;
mod sealed;
mod unclean;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need; the unclean epoch of the tests of history entries.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../history_reading/reading.rs"]
mod reading;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it; the senders of the
// tests of trust levels; and what the tests of replies share.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../replies/replying.rs"]
mod replying;
#[path = "../trust_levels/senders.rs"]
mod senders;

// The presence values of the tests of the agent directory, with the MCP client they use.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../presence_uploads/presence.rs"]
mod presence;
#[path = "../presence_directory/values.rs"]
mod values;
