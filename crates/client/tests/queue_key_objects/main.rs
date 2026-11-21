//! Queue key objects in the CLI, and tasks from service agents outside the group
//! (`docs/protocol/queues.md`, "Queue key", "Queue key object" and "Tasks";
//! `docs/protocol/membership.md`, "Clean epochs" and "Uploads").
//!
//! - The sender of each Commit uploads the queue key object of the new epoch with its link and
//!   wraps; any member device of an epoch, a member agent's among them, uploads it after its
//!   catch-up if the server has none. A device uploads none for an epoch it did not judge clean.
//! - A member device that finds the server's object with a `key` other than the `pk` it derives
//!   warns its user, once per epoch.
//! - A `service` device outside the group, in a stateless run of `skald enqueue`, seals to the `key`
//!   of the object of the channel's latest GroupInfo's epoch, only once that GroupInfo passes its
//!   checks, lists the object's `signer` as a member and the object's signature verifies with the
//!   certificate of `signer`, a valid device. The content is from the agent's entry, with its grant
//!   and no `counter`; the `deadline` is 7 days after enqueue when another account may claim the
//!   task. A refused run fails, saying why, and uploads no task; a token without `queues` gets
//!   `not-a-member`.
//!
//! As the tests of the key chain (`key_chain/main.rs`): the CLI runs on Ada's machine; Bob's and
//! Carol's devices are played by the tests with an MLS state of their own, so their runs make no
//! uploads of their own. The tests check the objects the server holds against the `pk` the played
//! devices derive from their history keys. An object Bob's device uploads, the test signs with its
//! key and sends in a `history` frame on its connection. The front keeps each queue key object Ada's
//! machine uploads, read from the frame's bytes, and can close her connection at the next one,
//! which the server never gets. The service agents, member or not, are those of the tests of member
//! agents (`service_members/main.rs`) and of stateless agents (`stateless_agents/main.rs`); the
//! relay gives a stateless agent other objects or GroupInfos as a compromised server would.
//!
//! The interfaces, as these tests use them, besides those of the tests named above and of queues
//! (`queues/main.rs`):
//! - On a device's connection, a `history` frame whose item is `[4, queue-key: bstr .cbor
//!   queue-key]`, answered with `history-stored` or `history-refused`.
//! - `GET /spaces/{space}/channels/{channel}/queue-keys/{epoch}`, with the bearer token and the
//!   device's proof: `200` with `{"queue_key": <bytes>}`, the object the server holds, as it got
//!   it, or `404` when it holds none. Bytes in JSON are base64url without padding.
//! - A warning of a wrong object: a line with "warning" holding the words `queue` and `key`.
//! - The stateless run: `skald enqueue --channel <channel> <text>` with the agent's secrets in its
//!   environment alone.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod objects;

mod stateless;
mod uploads;
mod warnings;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The queue requests and tasks of the tests of queues, with what an agent reads, as the tests of
// compact output have it.
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../queues/queuing.rs"]
mod queuing;

// The member agent, the stateless agent's scene and the service agents' commands and secrets, as
// their tests have them, with Ada's agents and the messages of the tests of gap detection they
// need.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../service_members/member.rs"]
mod member;
#[path = "../stateless_agents/scene.rs"]
mod scene;
#[path = "../service_agents/kit.rs"]
mod service_kit;
