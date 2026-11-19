//! Service agents as channel members: a long-lived service agent with a state directory is an MLS
//! member of its channels and exchanges messages there as a device does
//! (`docs/protocol/membership.md`, "Roster", "MLS profile", "Admitted devices", "Group changes" and
//! "Service device"; `docs/protocol/README.md`, "Service grant"; `docs/protocol/delivery.md`,
//! "Gap detection").
//!
//! - The creation command takes whether the agent is a member; the revision it signs then lists
//!   the agent's device in the `services` of its grant's channels, and the members' devices admit
//!   it there.
//! - A member agent reads its Commits and Welcomes from its mailbox, so the creation and token
//!   commands refuse its token without the receive permission.
//! - With its two secrets and a state directory, the client runs as the member `service` device:
//!   it gets into the channels' groups, added by a member's device or by itself, sends under `mls`
//!   with its grant and a counter, writes history entries and receives as a `client` device does,
//!   with gap detection.
//! - Its state is sealed under a key derived from its key secret: the directory alone opens
//!   nothing, and nothing goes to the key storage. The directory has the access rights the local
//!   state has.
//! - Once the device has been in a group, a run without its earlier state refuses, so its counters
//!   do not restart, and says to keep the state or replace the key.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): Ada's machine runs the CLI and
//! creates `acme`; Bob's device is played by the tests with an MLS state of its own, so they read
//! the content the agent sends, and a message the server drops is one Bob's device encrypts and
//! never sends. Ada's machine creates the service agent `ci/deploy-bot` as the tests of service
//! agents do (`service_agents/main.rs`); the agent runs on a machine of its own, with its key
//! secret and token in `SKALD_SERVICE_KEY` and `SKALD_SERVICE_TOKEN` and its state in a directory
//! that `SKALD_STATE_DIR` names and the client creates. A message the server drops from Ada's
//! mailbox is one whose entry the test deletes from the server's database, as a proxy that keeps
//! it from her catch-up would.
//!
//! The CLI, as these tests use it, beside the commands of the tests of service agents, of channel
//! messages and of reading the history:
//! - `skald create-service-agent <space> <owner>/<agent> --channel <channel>... --member`: creates a
//!   service agent that is a member of its channels. With a `--permission` set that lacks
//!   `receive`, it is refused, saying `receive`, with nothing registered, signed or stored.
//! - `skald create-service-token` for a member agent with a `--permission` set that lacks
//!   `receive` is refused, saying `receive`, with no token created and the earlier ones kept.
//! - With `SKALD_SERVICE_KEY` and `SKALD_SERVICE_TOKEN` set, `skald recv --json` and
//!   `skald send --all --channel <channel> <text>` run as the member agent. `skald recv --json`
//!   shows each message as a JSON object on a line of its own, with `msg_id`, `channel`, `from`,
//!   `device`, `device_name`, `level`, `text` and `after_missing`, and reports missing messages in
//!   a warning line that says `missing` and holds the channel's ID and the sending device's ID.
//! - Such a run with an empty state directory, after the agent was in a group, fails, sends
//!   nothing and says `state`.
//! - `skald history <channel>` on Ada's machine shows each message of the channel's history once,
//!   with its `from` and the sending device's `name`, `ci/deploy-bot` for the agent's.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod exchange;
mod links;
mod refusals;
mod runs;
mod start;
mod state;

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The messages Bob's device says, as the tests of gap detection have them, and the service agents'
// commands and secrets, as the tests of service agents have them.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../service_agents/kit.rs"]
mod service_kit;

mod member;
