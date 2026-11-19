//! Copies of a member's messages to service agents outside the channel's group
//! (`docs/protocol/delivery.md`, "Send"; `docs/protocol/README.md`, "Content" and "Service grant";
//! `docs/protocol/membership.md`, "Roster" and "Device checks"). A member device's message with an
//! `a2a` body also goes under `hpke`, with its counter for that recipient, to each `service` device
//! that:
//! - has an entry in the latest revision the sender accepted, but is not in the channel's
//!   `services` there;
//! - has a valid certificate;
//! - has a grant from the server that lists the channel, names the device, is signed by an admin's
//!   device in that revision and has not expired; the server gives the grants of live tokens with
//!   the receive permission;
//!
//! when the content has no `to`, or `to` names that entry. The sending device warns its user when
//! it starts sending a channel's copies to a `service` device it sent none of them before, naming
//! the agent and the channel. A copy the server refuses is not sent again, and the run says so.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! which creates `acme`; Bob's device is played by the tests with an MLS state of its own, in the
//! groups of the space's default channel and of a channel of both. Ada creates `ci/deploy-bot` in
//! the CLI, as the tests of service agents do (`service_agents/main.rs`), and the tests play the
//! service agent with what that creation printed: a test client that reads the agent's mailbox
//! with its token and the proof of its key, and opens each envelope with the key secret's keys.
//! The tests sign fixture revisions with the people's devices' keys, and the relay can give Ada's
//! machine the space's grants as a compromised server would.
//!
//! The interfaces, as these tests use them, beside those of the tests of sending, of channel
//! messages and of service agents:
//! - `skald send (--to <address> | --all) --channel <channel> <text>`, as the tests of the
//!   addressee say (`send_addressee/main.rs`); a warning that it starts sending a channel's copies
//!   to a service agent is a line saying `warning` with the agent's `owner/agent` and the
//!   channel's ID; a copy the server refuses is shown with its problem;
//! - the service agent's key secret, `SKALD_SERVICE_KEY`: base64url without padding of its
//!   `service-key` (`docs/protocol/membership.md`, "Service device");
//! - its mailbox: `POST /session/challenge` with its token as the bearer token, then `/ws` with an
//!   `auth` its key signs, as a device's connection; `sync`, catch-up and `ack`;
//! - `GET /spaces/{space}/service-grants`, as the server's tests of these copies say
//!   (`crates/server/tests/service_copies/main.rs`): `{"grants": [<grant>, ...]}`, each in
//!   base64url without padding.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

// The modules the tests of the channel history share (`history_common/mod.rs`): the server, its
// relay and front, people in spaces, played devices, revocations and the talk of channel messages.
#[path = "../history_common/mod.rs"]
mod history_common;
use history_common::*;

// The commands, secrets and tokens of the tests of service agents.
#[path = "../service_agents/kit.rs"]
mod agents;

mod reader;
mod scene;

mod copies;
mod none;
mod refused;
mod warnings;
