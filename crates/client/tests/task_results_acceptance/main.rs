//! Acceptance tests of task results on the sending device (`docs/protocol/queues.md`, "Tasks" and
//! "Results"; `docs/protocol/delivery.md`, "Device to agent" and "Receipts";
//! `docs/service-agents.md`).
//!
//! - The task's sending device drops a result unless checks 1–3 of "Results" pass, with a warning:
//!   its sending device is admitted to the channel; `task` is a task this device enqueued in the
//!   channel, `to` is that task's `from`, and `from` is that task's `to` if it has one; for `done`,
//!   the server gives the task as `done` with the result's `claim_id` and `generation`, claimed by
//!   the result's sending device.
//! - It takes the first `done` result that passes as the task's; later ones are not taken. A
//!   `failed` result that passes is handed over as a failed attempt.
//! - It hands a result to the `to` agent as a message: one line with the task's `#<ref>`, the
//!   outcome, the claiming agent and its trust level, in `skald recv` and `inbox`; `--json` has
//!   every field. The claiming device gets `read`.
//! - A `service` device that keeps no state between runs makes check 2 against the server: the task
//!   is in a queue of the channel, `to` is its agent, and the device in the task's `sched`, if any,
//!   sent the result.
//!
//! As the tests of claiming tasks (`task_claims/main.rs`): the CLI runs on Ada's machine, whose
//! agent claude enqueues in `~/work/acme`, and on her second machine, linked by hand, whose agent
//! `codex-v5d8tm` claims there; Bob's device, of another account the roster of `acme` lists, is
//! played by the tests with an MLS state of its own, in the talk's channel and in `acme`'s default
//! channel. Bob's device claims, completes and fails tasks through the server's requests, as the
//! tests of task states do (`task_states/main.rs`), and writes the results it sends itself, so a
//! test can send a result that a claiming CLI never would. The service agent `ci/deploy-bot` runs
//! without a disk as the tests of stateless agents run it (`stateless_agents/main.rs`); the tests
//! write its tasks, signed with its key secret's signing key and uploaded with its device's proof.
//!
//! The CLI, as these tests use it, beside the commands of the tests of claiming tasks and of
//! receiving:
//! - an agent's `skald recv` shows a result handed over to it as one line that holds the task's
//!   `#<ref>`, `task done` or `task failed`, the claiming agent's name and the trust level of the
//!   result's sending device as a word; `skald recv --json` gives it as one JSON object whose
//!   values hold the task's ID or `#<ref>`, `done` or `failed`, the claiming agent's address, the
//!   trust level, and the text of the response;
//! - the MCP tool `inbox` of an agent's `skald mcp` shows it as `skald recv` does;
//! - a result or message the device drops makes the person's next `skald recv` show a warning
//!   line, saying `warning`, that holds the result's sending device's ID, the task's ID or its
//!   `#<ref>`; neither run shows the result as handed over;
//! - the person's `skald recv` on the claiming machine shows `read` of the result, in a line that
//!   holds the result message's `msg_id` and the word `read`, as for any message
//!   (`read_receipts/main.rs`).

// Each test target uses a part of the shared modules and of what they bring in.
#![allow(dead_code, unused_imports)]

mod returns;

mod channels;
mod dropped;
mod repeated;
mod stateless;
mod taken;

// The modules the tests of the channel history share (`history_common/mod.rs`).
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The modules the tests of tasks after their claim share (`task_common/mod.rs`): Ada's agents,
// what an agent reads, the senders, the queue commands, the task states, the claims, Ada's second
// machine and the helpers of reading the history.
#[path = "../task_common/mod.rs"]
mod task_common;
use task_common::*;

// From other test targets, by name: the MCP client of the tests of the MCP server; the member
// service agent of the tests of service members; the presence helpers of the tests of presence
// uploads; the `read` statuses of the tests of read receipts; the machine of the tests of stateless
// agents; the commands and secrets of the tests of service agents; the presence values of the tests
// of the agent directory.
#[path = "../mcp_inbox/client.rs"]
mod client;
#[path = "../service_members/member.rs"]
mod member;
#[path = "../presence_uploads/presence.rs"]
mod presence;
#[path = "../read_receipts/receipts.rs"]
mod receipts;
#[path = "../stateless_agents/scene.rs"]
mod scene;
#[path = "../service_agents/kit.rs"]
mod service_kit;
#[path = "../presence_directory/values.rs"]
mod values;
