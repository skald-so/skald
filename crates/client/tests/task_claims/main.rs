//! Claiming, running and completing tasks in the CLI (`docs/protocol/queues.md`, "Claims and
//! leases", "Fencing", "Claim checks" and "Results"; `docs/dev/architecture.md`, "Queue";
//! `docs/protocol/README.md`, "Content").
//!
//! - `skald claim`, as an agent or the person, claims from a queue of a channel, by default
//!   `default`, stating the trust level `own` and that it takes tasks without a template. With no
//!   task it prints one short line.
//! - The device runs a task only once the claim checks 1–6 pass. A task that fails check 1 for a
//!   key the device lacks, or fails check 6, a `client` device releases and a member `service`
//!   device fails with retry; a task that fails any other check the device fails without retry,
//!   with a warning to its user. The agent gets none of them.
//! - A task that passes is extended at once with `started`, then handed to the agent as one line
//!   `[<from> · <device> · <level>] #<ref>: <text>`, as a message is; `--json` has every field.
//! - The agent extends the claim with its progress, completes it with a text and fails it with or
//!   without retry. On `claim-lost`, or once the lease passed on the device's clock without an
//!   answered extend, the command says the claim is lost, and no result goes out.
//! - After the server takes a complete or a fail, the device sends the `task-result`: from the
//!   claiming agent, to the task's `from`, `hops` the task's plus one, under `mls` when the task's
//!   `sender` is a member device of the current epoch, otherwise under `hpke` to `sender`, with a
//!   history entry. A result goes out even over the limit on `hops`, and the task counts as a
//!   message handed to the agent for the `hops` of its next send.
//! - A complete or fail waits in the local state before its request, so a run killed after it
//!   repeats it and the result goes out once.
//!
//! As the tests of trust levels (`trust_levels/main.rs`): the CLI runs on Ada's machine, whose
//! agents run in `~/work/acme`, bound to `acme`, and on her second machine, linked by hand, whose
//! `~/work/acme` is bound to `acme` too, where the agent `codex-v5d8tm` claims; Bob's device, of
//! another account the roster of `acme` lists, is played by the tests with an MLS state of its
//! own, and reads each result sent in the channel. Tasks are enqueued with `skald enqueue` on
//! Ada's first machine, or written by the tests: signed with the key of Ada's first device or
//! Bob's, sealed to the queue key of Bob's history key of the channel's current epoch, and
//! uploaded with that device's proof, as the tests of queues do (`queues/main.rs`). A `browser`
//! device of Ada's account is one the tests write to the server's database, as the tests of
//! history wraps do (`history_wraps/main.rs`), and so is its task. The member service agent
//! `ci/deploy-bot` runs on a machine of its own, as the tests of service members do
//! (`service_members/main.rs`).
//!
//! A compromised server or proxy is played in two ways: the relay answers a claim with a `claim`
//! the test builds, or the test changes the server's own copy of a task's scheduling fields in its
//! database (`tasks.template`, `tasks.deadline`, `tasks.attempts`, `tasks.queued_task`), so the
//! server hands out a task it would not. The tests move a lease's end, `tasks.lease_until`, in the
//! database instead of waiting.
//!
//! The CLI, as these tests use it, beside the commands of the tests of queues. `<task>` is a
//! task's `#<ref>`, with or without the `#`, or its ID.
//! - `skald claim [--channel <channel>] [--queue <name>] [--json]`: claims one task and, if it
//!   passes the claim checks, prints its line; `--json` prints it as one JSON object with `task`,
//!   `ref`, `from`, `level`, `text`, `claim_id`, `generation` and `attempt`. With no task it
//!   succeeds and prints one line saying `no task`.
//! - A task the device fails without retry makes the run show a warning line, saying `warning`,
//!   that holds the task's ID or `#<ref>`.
//! - `skald extend [--progress started|awaiting-approval] <task>`, `skald complete <task> <text>`
//!   and `skald fail [--retry] <task> <text>`: each succeeds once the server takes it. On a claim
//!   that is lost, each fails and says `lost`.
//! - A complete or fail the server has not answered waits in the local state; the device's next
//!   `skald recv` as that agent repeats it first, then sends the result.
//!
//! The server's interface, as these tests use it, each request proving its device as the tests of
//! queues do: `GET .../queues/{queue}/tasks/{task}`, with `state`, `progress`, `reason`, `attempts`
//! and `claim`, and `POST .../tasks/{task}/cancel`, as the server's tests of claims say
//! (`crates/server/tests/claims/main.rs`).

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod claiming;

mod checked;
mod completed;
mod crashed;
mod failed;
mod hops;
mod lost;
mod refused;
mod released;
mod sealed_to;
mod service;

// The modules the tests of the channel history share (`history_common/mod.rs`), the second
// machines of the tests of the history on new devices and the helpers of the tests of reading the
// history they need; the history entries of the tests of history entries; the browsers of the
// tests of history wraps.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_entries/entries.rs"]
mod entries;
#[path = "../history_reading/reading.rs"]
mod reading;
#[path = "../history_wraps/wraps.rs"]
mod wraps;

// Ada's agents, as the tests of handing messages over have them, with the messages of the tests of
// gap detection; what an agent reads, as the tests of compact output have it; the senders of the
// tests of trust levels; the queue commands and requests of the tests of queues; the task states
// and reads of the tests of task states.
#[path = "../agent_handover/agents.rs"]
mod agents;
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

// The member service agent of the tests of service members, with the commands and secrets of the
// tests of service agents.
#[path = "../service_members/member.rs"]
mod member;
#[path = "../service_agents/kit.rs"]
mod service_kit;
