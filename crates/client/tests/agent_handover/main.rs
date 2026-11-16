//! Handing messages over to agents: a recipient device hands each message to the agents it is for,
//! in the inbound mode `accept` (`docs/protocol/delivery.md`, "Hops" and "Device to agent").
//!
//! - A message goes to the agent in its content's `to` if it runs on this device; without `to`, to
//!   each agent of this device in the channel: each agent the device has run as in the channel's
//!   space. Content to a person goes to no agent; a receipt is never handed over.
//! - The device keeps each agent's messages in its local state until they are handed over, and
//!   marks a message handed over only after the handover: a crash in between repeats the handover
//!   and never loses it. Each handover carries `msg_id`; the next message from a sending device
//!   after missing ones is marked as following them.
//! - A message waits for an agent until its envelope's `ttl` ends, at most the mailbox retention;
//!   then the device drops it for that agent.
//! - The person's `skald recv` shows each message once, whether or not an agent got it.
//!
//! As the tests of channel messages (`channel_messages/main.rs`): the CLI runs on Ada's machine,
//! whose agents run in `~/work/acme`, bound to `acme` by a rule; Bob's device is played by the
//! tests with an MLS state of its own and sends the content they write, with `to` or without.
//!
//! The CLI, as these tests use it, beside the commands of the tests of channel messages and of gap
//! detection:
//! - `skald --agent <name> recv`, in a folder bound to a space: runs as the agent `<name>`, which
//!   from then on is one of the device's agents in that space; shows each message handed over to
//!   it, its text once and its `msg_id`, and marks it handed over once its output is written. The
//!   line that holds the `msg_id` of a message following missing ones says `missing`, as gap
//!   detection's tests say.

// Each test target uses a part of the shared modules.
#![allow(dead_code)]

mod killed;
mod kinds;
mod missing;
mod recipients;
mod waiting;

use std::path::{Path, PathBuf};

use skald_proto::{MessageId, RawEnvelope};

use crate::gaps::sends;
use crate::kit::RECV;
use crate::machine::{Finished, Machine};
use crate::talk::{Line, Talk, encrypted};
use crate::world::{ACME, ADD_RULE, AGENT, HOST, dir_of, path, run};

// The messages of the tests of gap detection, the kit of the tests of channel groups, the device
// the tests of receiving play over the delivery protocol, the device the tests of channel groups
// play, and the channel of the tests of channel messages.
#[path = "../gap_detection/gaps.rs"]
mod gaps;
#[path = "../channel_groups/kit.rs"]
mod kit;
#[path = "../receiving/peer.rs"]
mod peer;
#[path = "../channel_groups/played.rs"]
mod played;
#[path = "../channel_messages/talk.rs"]
mod talk;

// The server's tests' readers of the KeyPackages API's answers and the person's browser; the
// server, the fixtures and the front of the tests of sending, the machines, the relay and people in
// spaces.
#[path = "../../../server/tests/key_packages/answers.rs"]
mod answers;
#[path = "../../../server/tests/common/browser.rs"]
mod browser;
#[path = "../sign_in/cloud.rs"]
mod cloud;
#[path = "../sending/fixtures.rs"]
mod fixtures;
#[path = "../sending/front.rs"]
mod front;
#[path = "../sign_in/machine.rs"]
mod machine;
#[path = "../sign_in/relay.rs"]
mod relay;
#[path = "../roster_revisions/world.rs"]
mod world;

// The helpers of every client test, and finding keys in bytes.
#[path = "../common/mod.rs"]
mod common;
#[path = "../device_keys/keys.rs"]
mod keys;

/// The agents of Ada's device the tests run.
pub const CLAUDE: &str = "claude-h7w2kq";
pub const CODEX: &str = "codex-m3t9xa";

/// The folder of Ada's machine where her agents run, bound to `acme`.
pub const ACME_DIR: &str = "work/acme";

/// The talk of the tests of channel messages, Ada's machine with `~/work/acme` bound to `acme`.
pub struct Agents {
    pub talk: Talk,
    pub folder: PathBuf,
}

impl Agents {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, in `acme`'s channel;
    /// `~/work/acme` bound to `acme`.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let talk = Talk::new(&ids, &logins, false);
        let folder = dir_of(&talk.ada.machine, ACME_DIR);
        let bound = run(&talk.ada.machine, &[ADD_RULE, path(&folder), ACME]);
        assert!(bound.success, "not bound: {bound:?}");
        Agents { talk, folder }
    }

    /// Runs `skald --agent <agent> recv` in `~/work/acme`; fails unless it succeeds.
    #[track_caller]
    pub fn run_as(&self, agent: &str) -> Finished {
        run_as_in(&self.talk.ada.machine, &self.folder, agent)
    }

    /// The address of Ada's agent `agent` in `acme`.
    pub fn address(&self, agent: &str) -> String {
        format!("{ACME}/{}/{agent}@{HOST}", self.talk.ada.handle)
    }

    /// Bob's person address in `acme`.
    pub fn bob(&self) -> String {
        self.talk.address(&self.talk.played[0].person)
    }

    /// The `mls` envelope in which Bob's device says `line` in the channel; not sent.
    pub fn bob_says(&mut self, line: &Line) -> RawEnvelope {
        let at = self.talk.at();
        encrypted(&mut self.talk.played[0], at, at.1, line)
    }

    /// Bob's device sends `line` in the channel; fails unless the server stores it. Gives its
    /// `msg_id`.
    #[track_caller]
    pub fn bob_sends(&mut self, line: &Line) -> MessageId {
        let envelope = self.bob_says(line);
        sends(&self.talk, &envelope)
    }
}

/// Runs `skald --agent <agent> recv` on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn run_as_in(machine: &Machine, dir: &Path, agent: &str) -> Finished {
    let finished = machine.start_in(dir, &[AGENT, agent, RECV]).finish();
    assert!(
        finished.success,
        "{agent} in {}: {finished:?}",
        dir.display()
    );
    finished
}

/// Fails unless `shown` holds `text` once and the `msg_id` `id`.
#[track_caller]
pub fn assert_handed_once(shown: &str, text: &str, id: MessageId) {
    assert_eq!(shown.matches(text).count(), 1, "{text:?} not once: {shown}");
    assert!(
        shown.contains(&id.to_string()),
        "{text:?} without {id}: {shown}"
    );
}

/// Fails if `shown` holds `text`.
#[track_caller]
pub fn assert_not_handed(shown: &str, text: &str) {
    assert!(!shown.contains(text), "{text:?} handed over: {shown}");
}
