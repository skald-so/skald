//! A task's results on the device that enqueued it (`docs/protocol/queues.md`, "Results").
//!
//! - The device drops, with a warning, a result that fails checks 2 and 3 of "Results": one of a
//!   task it did not enqueue in the result's channel, one whose `to` is not the task's `from` or
//!   whose `from` is not the task's `to`, and a `done` one unless the server gives the task `done`
//!   with the result's `claim_id` and `generation`, claimed by the result's sending device.
//! - It takes the first `done` result that passes as the task's and drops any later one; each
//!   `failed` one that passes is shown as a failed attempt.
//! - It hands a result to its `to` agent as a message, one line `[<from> · <device> · <level>]
//!   #<ref> (task <outcome>): <text>` whose reference is the task's, and issues its `read`;
//!   `--json` gives it as an object of the type `result` with every field, `task` and `outcome`
//!   among them.
//! - A message whose content holds a `task` body is dropped with a warning.
//! - A stateless run of a service agent checks a result against the server: the task is in a queue
//!   of the result's channel, `to` is the run's agent, and the device in the task's `sched`, if
//!   any, sent the result.
//!
//! As the tests of claims (`task_claims/main.rs`): the CLI runs on Ada's machine, whose agents run
//! in `~/work/acme`, bound to `acme`, and on her second machine, linked by hand, where the agent
//! `codex-v5d8tm` claims; Bob's device is played by the tests with an MLS state of its own. Claude
//! enqueues with `skald enqueue`. Bob's device claims and completes through the server's requests,
//! as the tests of task states do (`task_states/main.rs`), and writes the results the tests craft
//! in the channel's group. The stateless service agent `ci/deploy-bot` runs as the tests of
//! stateless agents run it (`stateless_agents/main.rs`); its tasks are written by the tests and
//! uploaded with the proof of Ada's device, and Bob's device seals its results to it under `hpke`.
//! A proxy that changes a task's state is played by the relay.
//!
//! The CLI, as these tests use it, beside the commands of the tests of claims: a dropped result
//! gives a warning line naming its `msg_id` and the word `dropped`.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod results;

mod dropped;
mod stateless;
mod taken;

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
// tests of trust levels; the queue commands and requests of the tests of queues; the task states
// and reads of the tests of task states; the claims of the tests of claims.
#[path = "../agent_handover/agents.rs"]
mod agents;
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

// The stateless service agent of the tests of stateless agents, with the member agents and the
// commands and secrets of the tests of service agents it needs.
#[path = "../service_members/member.rs"]
mod member;
#[path = "../stateless_agents/scene.rs"]
mod scene;
#[path = "../service_agents/kit.rs"]
mod service_kit;
