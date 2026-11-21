//! Task states in the CLI (`docs/protocol/queues.md`, "Task states"; `docs/dev/architecture.md`,
//! "Queue").
//!
//! - `skald tasks` lists the tasks the server gives the device in a channel, or in one of its
//!   queues: each with its `#<ref>`, queue, state, the progress of its holder's last extend while
//!   `claimed`, its reason while `dead`, and its attempts. The person's lines are in full; an agent
//!   sees only the tasks it enqueued, in the short form of an agent's lines, which names no ID;
//!   `--json` gives every field.
//! - The person restarts a `dead` task and cancels one, by its `#<ref>` or ID; as an agent both
//!   are refused and change nothing. The server's refusals are shown: `not-allowed` for a member
//!   who is neither of the uploader's account nor an admin, `not-dead` for a restart of a task in
//!   another state, `not-cancellable` for a cancel of a `done` task.
//!
//! As the tests of queues (`queues/main.rs`): the CLI runs on Ada's machine, whose agents run in
//! `~/work/acme`, bound to `acme`; Bob's device, of another account the roster of `acme` lists, a
//! member who is no admin, is played by the tests. Ada's agents and the person enqueue the tasks
//! with `skald enqueue`; Bob's device claims them, stating `peer`, and extends, completes, fails
//! and releases its claims through the server's requests, as the server's tests of claims
//! (`crates/server/tests/claims/main.rs`) use them. In the test of a member who is no admin, Bob
//! and Carol are admins and a revision Ada's device signs makes Ada a member; Bob's device uploads
//! the tasks there as the server's tests of queues do, as the server does not open them. Time
//! passes for a task as in the server's tests of claims: every `timestamptz` the server keeps in a
//! row that names the task moves back.
//!
//! The CLI, as these tests use it, beside the commands of the tests of queues. `<channel>` is a
//! channel's ID, as `skald send --channel` takes it.
//! - `skald tasks [--channel <channel>] [--queue <name>] [--json]`: one line per task of the
//!   channel, or of its queue `<name>`, that the server gives the device, holding the task's
//!   reference `#<ref>`, made from its ID as a message's is from its `msg_id`, its queue's name,
//!   its state, its progress, `started` or `awaiting-approval`, while `claimed` after an extend,
//!   its reason, `lease-expired`, `failed`, `released` or `deadline-passed`, while `dead`, and its
//!   attempts as a number, each as a word. The person's line also holds the task's ID. As an agent,
//!   only the tasks that agent enqueued, and no ID. With `--json`, one JSON object per line and
//!   task: `task`, its ID; `ref`; `channel`; `queue`, the queue's ID; `queue_name`; `state`;
//!   `progress`; `reason`; `attempts`; and the device of its last claim.
//! - `skald restart-task [--channel <channel>] <task>` and `skald cancel-task [--channel <channel>]
//!   <task>`: `<task>` is `#<ref>`, `<ref>` or the task's ID. As an agent, each fails and makes no
//!   request that restarts or cancels a task. A refusal from the server fails the run, which names
//!   its problem as a word.
//!
//! The server's interface, as these tests use it, each request proving its device as
//! `/mailbox/send` does: those of the tests of queues, `GET .../queues/{queue}/tasks/{task}`,
//! `POST .../queues/{queue}/claims` and `POST .../queues/{queue}/tasks/{task}/extend`, `complete`,
//! `fail` and `release`, as the server's tests of claims use them.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod states;

mod dead;
mod listing;
mod others;
mod own;
mod restarts;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// What the tests of queues share.
#[path = "../queues/queuing.rs"]
mod queuing;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;

// A device's proof on a request, as the tests of presence uploads make it.
#[path = "../presence_uploads/presence.rs"]
mod presence;
