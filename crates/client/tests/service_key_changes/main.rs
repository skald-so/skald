//! Replacing and removing a service agent (`docs/service-agents.md`; `docs/protocol/membership.md`,
//! "Roster", "Device checks" and "Service device"; `docs/protocol/README.md`, "Service grant").
//!
//! - Replacing the key, by an admin of the space for a service agent of the admin's own account: a
//!   new `service` device is created as at the agent's creation and takes the old one's place in
//!   the agent's entry and in `services` in the next revision; then the account revokes the old
//!   device, which ends its tokens and takes it out of the groups. The new key secret, whose anchor
//!   is that revision, and a new token are written once.
//! - Removing, by any admin of the space: the next revision drops the entry and the device from
//!   `services`, which ends the device's tokens; the device's own account then revokes it.
//! - A revision that lists a device revoked later stays acceptable to a device that walks past it.
//! - A revision an admin's device signs drops the entries and `services` of revoked `service`
//!   devices and of the accounts it removes; the server refuses a new revision that lists a revoked
//!   `service` device or one on a non-member's account (`crates/server/tests/service_revisions`).
//!
//! As the tests of a service agent without a persistent disk (`stateless_agents/main.rs`) and of
//! member agents (`service_members/main.rs`): Ada's machine runs the CLI and creates `acme` and
//! `ci/deploy-bot`; Bob's device is played by the tests, or Bob is only an account. Where a test
//! says so, Ada's second machine, linked by hand as in the tests of the history on new devices,
//! runs the CLI too, and a `service` device of Bob's account, certified by his device, is set up in
//! the server's database, its entry in a revision Ada's device signs. The agent runs on machines of
//! its own with only its two secrets set, and a state directory if it is a member.
//!
//! The CLI, as these tests use it, beside the commands of the tests of service agents, of member
//! agents, of removing members and of revoking devices:
//! - `skald replace-service-key <space> <owner>/<agent> --channel <channel>...`: replaces the key of
//!   the service agent of this device's account; `--expires` and `--permission` as at its creation
//!   set its new grant's expiry and new token's permissions. On success, stdout is exactly the two
//!   lines `SKALD_SERVICE_KEY=<key secret>` and `SKALD_SERVICE_TOKEN=<token>`, in either order. For
//!   an agent of another account it is refused, saying `account`, with nothing on stdout and
//!   nothing registered, signed, stored or revoked.
//! - `skald remove-service-agent <space> <owner>/<agent>`: on a device of an admin of the space,
//!   signs the space's next revision without the agent's entry and without its device in any
//!   channel's `services`, as `skald create-channel` signs a revision. It may also revoke the
//!   device when it is of this device's account; if it does not, `skald revoke-device` does.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod changes;

mod accounts;
mod refusals;
mod removing;
mod replacing;
mod revoking;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices, revocations and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The service agents' commands and secrets, as the tests of service agents have them; the member
// agents and their runs, as the tests of member agents have them; the stateless agents and their
// runs, as the tests of agents without a persistent disk have them.
#[path = "../service_members/member.rs"]
mod member;
#[path = "../stateless_agents/scene.rs"]
mod scene;
#[path = "../service_agents/kit.rs"]
mod service_kit;

// Ada's second machine, linked by hand, as the tests of the history on new devices have it, with
// the helpers of the tests of reading the history it needs.
#[path = "../history_devices/devices.rs"]
mod devices;
#[path = "../history_reading/reading.rs"]
mod reading;

// Ada's agents and what an agent's output shows of a sender's level, which the stateless agents'
// runs use, as the tests of handing messages over and of trust levels have them, with the modules
// they need.
#[path = "../agent_handover/agents.rs"]
mod agents;
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../compact_output/lines.rs"]
mod lines;
#[path = "../trust_levels/senders.rs"]
mod senders;
