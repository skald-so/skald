//! What the tests share: the talk of Ada and Bob with the member service agent `ci/deploy-bot` in
//! the talk's channel, the machines the agent runs on, their runs and what they show, and what
//! Bob's device reads of the agent.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use skald_crypto::check_grant;
use skald_proto::{Cbor as _, ChannelId, Content, DeviceId, MessageId, RawEnvelope, ServiceKey};

use crate::fixtures::send;
use crate::kit::{RECV, latest, received};
use crate::machine::{Finished, Machine, STATE_DIR};
use crate::service_kit::{BOT, Secrets, create_agent, key_of, secrets_of, service_of};
use crate::talk::{Talk, a2a, messages_sent};
use crate::world::{ACME, HOST, now, run, space_certs};

/// The option of the creation command that makes the agent a member of its channels.
pub const MEMBER: &str = "--member";

/// The option of `skald recv` that shows each message as JSON.
pub const JSON: &str = "--json";

/// What a refused run of a member agent says when its earlier state is missing.
pub const STATE: &str = "state";

/// The directory of an agent's machine that its state goes to; the client creates it.
const AGENT_STATE: &str = "agent-state";

/// How many rounds of runs an agent gets to get into the channel's group.
const ROUNDS: usize = 4;

/// A machine the service agent runs on: of its own, with settings of its runs, its state in a
/// directory the client creates.
pub struct Runner {
    pub machine: Machine,
    /// The state directory.
    pub state: PathBuf,
}

impl Runner {
    /// A new machine whose runs have the settings `vars`, by name.
    pub fn with(vars: &[(&str, &str)]) -> Self {
        let machine = Machine::new();
        let state = machine.root().join(AGENT_STATE);
        let mut machine = machine.with_env(STATE_DIR, &state);
        for (name, value) in vars {
            machine = machine.with_env(name, *value);
        }
        Runner { machine, state }
    }

    /// A new machine whose runs have both of `secrets`, as their environment variables.
    pub fn of(secrets: &Secrets) -> Self {
        Self::with(&secrets.variables())
    }

    /// A new machine whose runs have the settings `vars`, its state directory a copy of `from`'s.
    pub fn copy_of(from: &Runner, vars: &[(&str, &str)]) -> Self {
        let runner = Self::with(vars);
        copy_dir(&from.state, &runner.state);
        runner
    }

    /// Runs `skald recv --json`.
    pub fn recv(&self) -> Finished {
        run(&self.machine, &[RECV, JSON])
    }

    /// As [`Runner::recv`]; fails unless it succeeds.
    #[track_caller]
    pub fn received(&self) -> Finished {
        let finished = self.recv();
        assert!(finished.success, "the agent's recv failed: {finished:?}");
        finished
    }

    /// Runs `skald send --all --channel <channel> <text>`.
    pub fn send(&self, channel: ChannelId, text: &str) -> Finished {
        send(&self.machine, channel, text)
    }

    /// Every file in the state directory, at any depth.
    pub fn files(&self) -> Vec<PathBuf> {
        let mut files = self.machine.files();
        files.retain(|file| file.starts_with(&self.state));
        files
    }
}

/// Copies the directory `from`, with everything in it, to `to`, which must not exist.
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir(to).unwrap_or_else(|err| panic!("create {}: {err}", to.display()));
    let entries = fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display()));
    for entry in entries {
        let entry = entry.expect("a directory entry");
        let (path, target) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().expect("a file type").is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap_or_else(|err| panic!("copy {}: {err}", path.display()));
        }
    }
}

/// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, in `acme`, and
/// the secrets of `ci/deploy-bot`, which Ada's machine creates for the talk's channel with `extra`;
/// the creation must succeed.
pub fn talk_with_bot(ids: [u64; 2], logins: [&str; 2], extra: &[&str]) -> (Talk, Secrets) {
    let talk = Talk::new(&ids, &logins, false);
    let created = create_agent(&talk.ada.machine, ACME, BOT, &[talk.channel], extra);
    (talk, secrets_of(&created))
}

/// The address of `ci/deploy-bot` in `acme`, `acme/ci/deploy-bot@<server>`.
pub fn bot_address() -> String {
    format!("{ACME}/{BOT}@{HOST}")
}

/// The talk of Ada and Bob in `acme`, and the service agent `ci/deploy-bot` that Ada's machine
/// created as a member of the talk's channel.
pub struct Bot {
    pub talk: Talk,
    pub secrets: Secrets,
    pub key: ServiceKey,
    /// Its `service` device.
    pub device: DeviceId,
    /// Its address, `acme/ci/deploy-bot@<server>`.
    pub address: String,
    /// The machine it runs on, with both secrets.
    pub runner: Runner,
}

impl Bot {
    /// The talk of Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones; Ada's
    /// machine creates `ci/deploy-bot` as a member of the talk's channel, with the creation's
    /// default permissions, which must succeed.
    pub fn created(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let (talk, secrets) = talk_with_bot(ids, logins, &[MEMBER]);
        let (device, _) = service_of(talk.cloud(), &talk.ada);
        let key = key_of(&secrets.key);
        let address = bot_address();
        let runner = Runner::of(&secrets);
        Bot {
            talk,
            secrets,
            key,
            device,
            address,
            runner,
        }
    }

    /// The talk's channel.
    pub fn channel(&self) -> ChannelId {
        self.talk.channel
    }

    /// Runs the agent's `skald recv`, then Ada's, then Bob's device takes in its mailbox, in rounds,
    /// until Bob's device holds the agent in the channel's group; then the agent runs once more, to
    /// take in what a member's device sent it. Fails if a run fails, or after [`ROUNDS`] rounds.
    /// Gives what Ada's runs showed.
    #[track_caller]
    pub fn brought_in(&mut self) -> Vec<Finished> {
        let mut ada_runs = Vec::new();
        for _ in 0..ROUNDS {
            self.runner.received();
            ada_runs.push(received(&self.talk.ada.machine));
            self.bob_takes_in();
            if self.bob_holds_the_agent() {
                self.runner.received();
                return ada_runs;
            }
        }
        panic!(
            "the agent {} is not in the channel's group after {ROUNDS} rounds; Ada's runs: {ada_runs:?}",
            self.device
        );
    }

    /// Bob's device takes in its mailbox.
    pub fn bob_takes_in(&mut self) {
        self.talk.played[0].take_in(&self.talk.world.cloud);
    }

    /// Whether Bob's device holds the agent's device in the channel's group.
    pub fn bob_holds_the_agent(&self) -> bool {
        self.talk.played[0]
            .members(self.channel())
            .contains(&self.device)
    }

    /// Ada's machine sends `text` in the channel, and the agent's next run shows it once, from Ada's
    /// address and device; fails unless both runs succeed.
    #[track_caller]
    pub fn ada_reaches_the_agent(&self, text: &str) {
        let (ada, channel) = (&self.talk.ada, self.channel());
        let sent = send(&ada.machine, channel, text);
        assert!(sent.success, "Ada's send failed: {sent:?}");
        let got = self.runner.received();
        shown_once(&got, text, (channel, &self.talk.address(ada), ada.device));
    }

    /// The agent sends `text` in the channel; fails unless the run succeeds and the front saw it
    /// send one message envelope there. Gives that envelope.
    #[track_caller]
    pub fn says(&self, text: &str) -> RawEnvelope {
        let mark = self.talk.front.sent().len();
        let sent = self.runner.send(self.channel(), text);
        assert!(sent.success, "the agent's send failed: {sent:?}");
        match messages_sent(&self.talk.front, self.channel(), mark).as_slice() {
            [one] => one.envelope.clone(),
            other => panic!("not one message of the agent in the channel: {other:?}; {sent:?}"),
        }
    }

    /// The application messages of the channel Bob's device decrypted from the agent's device, in
    /// order, each with its envelope's `msg_id` and its content.
    #[track_caller]
    pub fn bob_read_from_the_agent(&self) -> Vec<(MessageId, Content)> {
        let bob = &self.talk.played[0];
        bob.read_in(self.channel())
            .into_iter()
            .filter(|(_, read)| read.sender == self.device)
            .map(|(msg_id, read)| {
                let content = Content::from_cbor(&read.plaintext)
                    .unwrap_or_else(|err| panic!("{msg_id} holds no content: {err:?}"));
                (msg_id, content)
            })
            .collect()
    }

    /// As [`assert_granted`], of the agent's device.
    #[track_caller]
    pub fn assert_granted(&self, content: &Content) {
        assert_granted(&self.talk, self.device, content);
    }
}

/// Fails unless `content` carries a grant that passes the checks of "Service grant" as Bob's device
/// in `talk` makes them now, against the latest revision and the space's certificates: of
/// `device`, for the talk's channel, signed by an admin's device, not expired.
#[track_caller]
pub fn assert_granted(talk: &Talk, device: DeviceId, content: &Content) {
    let grant = (content.grant.as_ref()).unwrap_or_else(|| panic!("no grant: {content:?}"));
    let (cloud, bob) = (talk.cloud(), &talk.played[0].person);
    let roster = latest(cloud, &bob.token, talk.space.id);
    let certs = space_certs(cloud, &bob.token, talk.space.id);
    check_grant(
        grant,
        (device, talk.channel),
        &roster,
        &certs,
        &[],
        (bob.account, &bob.master),
        now(),
    )
    .unwrap_or_else(|err| panic!("Bob's device refuses the grant: {err}; {content:?}"));
}

/// The messages `finished`, a run of `skald recv --json`, shows: the JSON objects with a `text` on
/// its stdout's lines.
pub fn messages(finished: &Finished) -> Vec<Value> {
    finished
        .stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|value| value.get("text").is_some())
        .collect()
}

/// The one message of `finished`, a run of `skald recv --json`, with `text`, which must come from
/// the device `device` with `from` in `channel`; fails unless it shows exactly one.
#[track_caller]
pub fn shown_once(
    finished: &Finished,
    text: &str,
    (channel, from, device): (ChannelId, &str, DeviceId),
) -> Value {
    let shown: Vec<Value> = messages(finished)
        .into_iter()
        .filter(|message| message["text"] == text)
        .collect();
    let [message] = shown.as_slice() else {
        panic!("{text:?} is not shown once: {finished:?}");
    };
    assert_eq!(message["channel"], channel.to_string(), "{message}");
    assert_eq!(message["from"], from, "{message}");
    assert_eq!(message["device"], device.to_string(), "{message}");
    message.clone()
}

/// Fails if `finished` shows `text` anywhere.
#[track_caller]
pub fn assert_not_shown(finished: &Finished, text: &str) {
    assert!(
        !finished.shown().contains(text),
        "{text:?} is shown: {finished:?}"
    );
}

/// The text parts of the A2A message in `content`.
#[track_caller]
pub fn texts_of(content: &Content) -> Vec<String> {
    a2a(content).1
}
