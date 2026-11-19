//! What the tests share: Ada's machine in `clienta` and `clientb`, each with a channel of hers whose
//! group holds Bob's played device, `~/work/clientA` bound to `clienta`, the runs of `skald send`
//! there and what Bob's device reads of them.

use std::path::{Path, PathBuf};

use serde_json::Value;
use skald_proto::{Body, Cbor as _, ChannelId, Content, MessageId, Role};

use crate::fixtures::{SEND, sent_after};
use crate::kit::{Space, assert_members, bind_client_a, create_space, received, with_channel};
use crate::machine::{Finished, Machine};
use crate::talk::{Talk, is_message};
use crate::world::{ADD_RULE, AGENT, CLIENT_A, CLIENT_B, HOST, dir_of, path, run};

/// The flags of `skald send`.
pub const TO: &str = "--to";
pub const ALL: &str = "--all";
pub const CHANNEL: &str = "--channel";

/// The agent Ada's device runs as in `~/work/clientA`.
pub const ADA_AGENT: &str = "claude-a9f3qe";

/// Two agents of Bob's account, by name.
pub const BOB_CLAUDE: &str = "claude-r4n8vz";
pub const BOB_CODEX: &str = "codex-k2p7wd";

/// A message Bob's device read in a channel: its `msg_id`, `from` and `to` in text form, and its
/// text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub msg_id: MessageId,
    pub from: String,
    pub to: Option<String>,
    pub text: String,
}

/// Ada's machine, which created `clienta`, the talk's space, with the talk's channel, and
/// `clientb` with a channel of its own; Bob's played device is in the groups of all four
/// channels. `~/work/clientA` is bound to `clienta`.
pub struct Clients {
    pub talk: Talk,
    /// `clientb`, and its channel beside the default one.
    pub client_b: Space,
    pub b_channel: ChannelId,
    /// `~/work/clientA` on Ada's machine.
    pub folder_a: PathBuf,
}

impl Clients {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let mut talk = Talk::in_space(CLIENT_A, &ids, &logins, false);
        let cloud = &talk.world.cloud;
        let client_b = create_space(cloud, &talk.ada, CLIENT_B);
        let b_channel = with_channel(
            cloud,
            client_b.id,
            &talk.ada,
            &[(&talk.played[0].person, Role::Member)],
        );
        received(&talk.ada.machine);
        let devices = [talk.ada.device, talk.played[0].device()];
        let bob = &mut talk.played[0];
        bob.take_in(cloud);
        for group in [client_b.default, b_channel] {
            assert_members(&bob.members(group), &devices);
        }
        let folder_a = bind_client_a(&talk.ada.machine);
        Clients {
            talk,
            client_b,
            b_channel,
            folder_a,
        }
    }

    /// Every channel of both spaces: `clienta`'s default one and the talk's, then `clientb`'s
    /// default one and its other.
    pub fn channels(&self) -> [ChannelId; 4] {
        [
            self.talk.space.default,
            self.talk.channel,
            self.client_b.default,
            self.b_channel,
        ]
    }

    /// The address of Bob's agent `agent` in the space `space`.
    pub fn bob_agent(&self, space: &str, agent: &str) -> String {
        format!(
            "{space}/{}/{agent}@{HOST}",
            self.talk.played[0].person.handle
        )
    }

    /// Bob's handle.
    pub fn bob(&self) -> String {
        self.talk.played[0].person.handle.to_string()
    }

    /// `rel` under the home of Ada's machine, such as `work/clientB`, bound to the space `space`.
    pub fn bound(&self, rel: &str, space: &str) -> PathBuf {
        let folder = dir_of(&self.talk.ada.machine, rel);
        let bound = run(&self.talk.ada.machine, &[ADD_RULE, path(&folder), space]);
        assert!(bound.success, "not bound: {bound:?}");
        folder
    }

    /// The machine the CLI runs on: Ada's.
    pub fn ada(&self) -> &Machine {
        &self.talk.ada.machine
    }

    /// Runs `skald send <args>` on Ada's machine as the person, from her home.
    pub fn person_sends(&self, args: &[&str]) -> Finished {
        sends(self.ada(), None, None, args)
    }

    /// Runs `skald send <args>` on Ada's machine as the person in `dir`.
    pub fn person_sends_in(&self, dir: &Path, args: &[&str]) -> Finished {
        sends(self.ada(), Some(dir), None, args)
    }

    /// Runs `skald --agent <agent> send <args>` on Ada's machine in `dir`.
    pub fn agent_sends_in(&self, dir: &Path, agent: &str, args: &[&str]) -> Finished {
        sends(self.ada(), Some(dir), Some(agent), args)
    }

    /// How many `send` frames the front has seen so far.
    pub fn mark(&self) -> usize {
        self.talk.front.sent().len()
    }

    /// Fails if the front saw a message envelope, of any channel, after its first `from` sends.
    #[track_caller]
    pub fn assert_no_message_sent(&self, from: usize, what: &str) {
        let messages: Vec<_> = (sent_after(&self.talk.front, from).into_iter())
            .filter(|sent| is_message(sent.envelope.envelope()))
            .map(|sent| sent.envelope.envelope().header.clone())
            .collect();
        assert!(messages.is_empty(), "{what}: sent {messages:?}");
    }

    /// Bob's device takes in its mailbox; gives every message it read so far in `channel`.
    pub fn bob_reads(&mut self, channel: ChannelId) -> Vec<Read> {
        let bob = &mut self.talk.played[0];
        bob.take_in(&self.talk.world.cloud);
        (bob.read_in(channel).into_iter())
            .map(|(msg_id, decrypted)| {
                let content = Content::from_cbor(&decrypted.plaintext)
                    .unwrap_or_else(|err| panic!("{msg_id} holds no content: {err:?}"));
                Read {
                    msg_id,
                    from: content.from.to_string(),
                    to: content.to.as_ref().map(ToString::to_string),
                    text: text_of(&content),
                }
            })
            .collect()
    }

    /// Fails unless Bob's device, once it takes in its mailbox, has read no message in any channel
    /// of both spaces.
    #[track_caller]
    pub fn assert_bob_read_nothing(&mut self) {
        for channel in self.channels() {
            let read = self.bob_reads(channel);
            assert!(read.is_empty(), "read in {channel}: {read:?}");
        }
    }

    /// Person addresses, which an agent's address in a send does not take, each with what it is:
    /// Bob's and Ada's, full, `owner/user` and in uppercase.
    pub fn person_addresses(&self) -> [(&'static str, String); 4] {
        let (bob, ada) = (self.bob(), self.talk.ada.handle.to_string());
        [
            ("bob, full", format!("{CLIENT_A}/{bob}/user@{HOST}")),
            ("bob, short", format!("{bob}/user")),
            ("bob, uppercase", format!("{bob}/USER")),
            ("ada, full", format!("{CLIENT_A}/{ada}/user@{HOST}")),
        ]
    }

    /// Addresses a send to an agent in `clienta` does not take, each with what it is and the words
    /// its refusal says: invalid ones, one of another space and one of another server.
    pub fn invalid_addresses(&self) -> Vec<(&'static str, String, &'static [&'static str])> {
        const INVALID: &[&str] = &["address"];
        let bob = self.bob();
        let invalid = [
            BOB_CLAUDE.to_owned(),
            format!("{bob}/"),
            format!("/{BOB_CLAUDE}"),
            format!("{bob}/claude_r4n8vz"),
            format!("{CLIENT_A}//{BOB_CLAUDE}@{HOST}"),
            format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@{HOST}:8443"),
            format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@"),
            format!("{CLIENT_A}/{bob}/{BOB_CLAUDE} x@{HOST}"),
        ];
        let mut addresses: Vec<_> = (invalid.into_iter())
            .map(|address| ("invalid", address, INVALID))
            .collect();
        addresses.push((
            "another space",
            format!("{CLIENT_B}/{bob}/{BOB_CLAUDE}@{HOST}"),
            &["space"],
        ));
        addresses.push((
            "another server",
            format!("{CLIENT_A}/{bob}/{BOB_CLAUDE}@elsewhere.example"),
            &["server"],
        ));
        addresses
    }
}

/// Runs `skald [--agent <agent>] send <args>` on `machine`, in `dir` if any.
fn sends(machine: &Machine, dir: Option<&Path>, agent: Option<&str>, args: &[&str]) -> Finished {
    let mut all: Vec<&str> = Vec::new();
    if let Some(agent) = agent {
        all.extend([AGENT, agent]);
    }
    all.push(SEND);
    all.extend(args);
    match dir {
        Some(dir) => machine.start_in(dir, &all).finish(),
        None => machine.start(&all).finish(),
    }
}

/// The text parts of the A2A message in `content`, joined.
fn text_of(content: &Content) -> String {
    let message: Value = match &content.body {
        Some(Body::A2a(json)) => serde_json::from_str(json)
            .unwrap_or_else(|err| panic!("no A2A message in JSON: {err}: {json}")),
        other => panic!("a body other than a2a: {other:?}"),
    };
    let parts = message.get("parts").and_then(Value::as_array);
    (parts.into_iter().flatten())
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

/// Fails unless `read` is one message, of `text`, with `to` `to`; gives it.
#[track_caller]
pub fn assert_one(read: &[Read], text: &str, to: Option<&str>) -> Read {
    let of_text: Vec<&Read> = read.iter().filter(|one| one.text == text).collect();
    let [one] = of_text.as_slice() else {
        panic!("{text:?} not read once: {read:?}");
    };
    assert_eq!(one.to.as_deref(), to, "the `to` of {text:?}");
    (*one).clone()
}

/// Fails unless `finished` failed and shows each of `words`, in any case.
#[track_caller]
pub fn assert_refused(finished: &Finished, words: &[&str], what: &str) {
    assert!(!finished.success, "{what}: not refused: {finished:?}");
    let shown = finished.shown().to_lowercase();
    for word in words {
        assert!(
            shown.contains(&word.to_lowercase()),
            "{what}: the refusal does not say {word:?}: {finished:?}"
        );
    }
}
