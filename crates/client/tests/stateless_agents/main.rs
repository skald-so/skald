//! A service agent without a persistent disk (`docs/service-agents.md`;
//! `docs/protocol/membership.md`, "Anchors", "Service device" and "Roster";
//! `docs/protocol/README.md`, "Service grant"; `docs/protocol/delivery.md`, "Send" and "Mailbox"):
//! a one-shot run with only the key secret and the token in its environment.
//!
//! - `skald send` and `skald recv` run as the agent's `service` device with their state in memory
//!   only: no helper, no lock, nothing in the key storage, the local state or temporary files.
//! - A send seals one `hpke` envelope to each member device of the channel's latest GroupInfo, once
//!   that GroupInfo passes its checks and every member it lists is admitted; the content is from
//!   the agent's entry, with the grant and no `counter`. The run fails unless the server stores
//!   every envelope within it.
//! - `skald recv` writes each message to its output before it acks it, so a run that dies between
//!   the two shows the message again at the next one.
//! - A run starts its space's roster from the latest revision an anchors object of a valid `client`
//!   device of its account names, when the revisions from the key secret's anchor up to it pass
//!   through every other such anchored revision; otherwise from the key secret's anchor, saying so.
//!   It accepts the revisions up to that start without the check that their signer is not revoked.
//! - A run refuses, saying why, when the latest revision it accepted has no entry with its device,
//!   or when its grant fails its checks against that revision; it then says a new token is needed.
//! - No person sets the inbound mode of the run's agent, so it takes every message; a member's agent
//!   in `refuse` takes the run's message only once its allowlist admits the agent.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): Ada's machine runs the CLI and
//! creates `acme`; Bob's device is played by the tests with an MLS state of its own. Ada's machine
//! creates the service agent `ci/deploy-bot` for the talk's channel as the tests of service agents
//! do (`service_agents/main.rs`), not as a member. The agent runs on a machine of its own with an
//! empty home and temporary directory, `SKALD_SERVICE_KEY` and `SKALD_SERVICE_TOKEN` set, and
//! neither `SKALD_STATE_DIR` nor one-shot mode; its key storage is the secrets file, as on every
//! machine of the tests, so a run that kept a secret would leave a file. Its working directory is
//! its home. The tests sign fixture revisions and anchors objects with the people's devices' keys,
//! and the relay gives the agent revisions, anchors objects, devices, grants and GroupInfo as a
//! compromised server would.
//!
//! The CLI, as these tests use it, beside the commands of the tests of service agents:
//! - with both variables set, `skald send --all --channel <channel> <text>` and `skald recv --json`
//!   run as the agent; `skald recv --json` shows each message as a JSON object on a line of its
//!   own, with `channel`, `from`, `device`, `device_name`, `level` and `text`, as for a member agent
//!   (`service_members/main.rs`);
//! - a warning that a revision has no effect names it as `revision <number>` and names its signer's
//!   device ID; a warning that the run starts from its key secret's anchor says `anchor` and names
//!   the anchor as `revision <number>`;
//! - a run refused for its grant names `create-service-token`, the command of a new token; one
//!   refused for a revision without its entry names that revision as `revision <number>`; one whose
//!   envelope the server refuses names the problem.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod exchange;
mod inbound;
mod refusals;
mod runs;
mod start;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices, revocations and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The service agents' commands and secrets, as the tests of service agents have them, and what a
// run of an agent shows, as the tests of member agents have it.
#[path = "../service_members/member.rs"]
mod member;
#[path = "../service_agents/kit.rs"]
mod service_kit;

// Ada's second machine, linked by hand, as the tests of the history on new devices have it, with
// the helpers of the tests of reading the history it needs.
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// Ada's agents and what an agent's output shows of a sender's level, as the tests of handing
// messages over and of trust levels have them, with the modules they need.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../trust_levels/senders.rs"]
mod senders;

mod scene;
