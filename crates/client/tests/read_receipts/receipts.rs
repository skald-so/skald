//! What the tests share: A, the agent of Ada's second machine, sends to claude, B's agent on Ada's
//! first machine; what A shows of `read`, and the receipts C, Bob's played device, reads in the
//! channel.

use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use skald_proto::{Body, Cbor as _, ChannelId, Content, DeviceId, MessageId, Receipt};

use crate::agents::CLAUDE;
use crate::cloud::WAIT;
use crate::fixtures::{CHANNEL, SEND, TIMEOUT, TIMEOUT_SETTING, TO};
use crate::kit::{Change, RECV, latest, received, revise};
use crate::lines::{JSON_FLAG, json_objects};
use crate::machine::{Finished, Machine, Run};
use crate::played::Played;
use crate::senders::{LAPTOP_AGENT, Senders, WAIT_FLAG, holds_word};
use crate::talk::messages_sent;
use crate::world::{ACME, AGENT, HOST};

/// The status the CLI shows once the `to` agent has the message.
pub const READ: &str = "read";

/// The lines of `shown`, a person's output, that report `read` of `msg_id`: they hold its
/// `msg_id` and the word [`READ`].
pub fn read_lines(shown: &str, msg_id: MessageId) -> usize {
    let id = msg_id.to_string();
    (shown.lines())
        .filter(|line| line.contains(&id) && holds_word(line, READ))
        .count()
}

/// The `read` status objects of `msg_id` among the JSON objects of `stdout`.
pub fn read_objects(stdout: &str, msg_id: MessageId) -> Vec<Value> {
    let id = msg_id.to_string();
    (json_objects(stdout).into_iter())
        .filter(|object| object["msg_id"].as_str() == Some(id.as_str()))
        .filter(|object| object["status"].as_str() == Some(READ))
        .collect()
}

/// Runs `skald recv` on `machine` every half second until a run shows `read` of `msg_id`, for up
/// to [`WAIT`]; fails if a run fails or none does. Gives what the runs showed together.
#[track_caller]
pub fn shows_read(machine: &Machine, msg_id: MessageId) -> String {
    let deadline = Instant::now() + WAIT;
    let mut shown = received(machine).shown();
    while read_lines(&shown, msg_id) == 0 {
        assert!(
            Instant::now() < deadline,
            "no {READ} of {msg_id} shown: {shown}"
        );
        thread::sleep(Duration::from_millis(500));
        shown += &received(machine).shown();
    }
    shown
}

/// As [`shows_read`]; fails unless the runs showed `read` of `msg_id` once, and the next run shows
/// it no more.
#[track_caller]
pub fn shows_read_once(machine: &Machine, msg_id: MessageId) {
    let shown = shows_read(machine, msg_id);
    assert_eq!(read_lines(&shown, msg_id), 1, "{READ} not once: {shown}");
    let next = received(machine).shown();
    assert_eq!(read_lines(&next, msg_id), 0, "{READ} again: {next}");
}

/// Fails if `machine`'s next `skald recv` shows `read` of `msg_id`.
#[track_caller]
pub fn shows_no_read(machine: &Machine, msg_id: MessageId, when: &str) {
    let shown = received(machine).shown();
    assert_eq!(read_lines(&shown, msg_id), 0, "{when}: {shown}");
}

/// A receipt message a played device read in a channel.
#[derive(Debug, Clone)]
pub struct Got {
    /// The `msg_id` of its envelope.
    pub msg_id: MessageId,
    /// The device that sent it.
    pub sender: DeviceId,
    pub content: Content,
    pub receipt: Receipt,
}

impl Got {
    /// Whether it reports on `msg_id`.
    pub fn lists(&self, msg_id: MessageId) -> bool {
        self.receipt.messages.contains(&msg_id)
    }
}

/// The receipt messages `played` read in `channel` so far, in order.
pub fn receipts_in(played: &Played, channel: ChannelId) -> Vec<Got> {
    (played.read_in(channel).into_iter())
        .filter_map(|(msg_id, decrypted)| {
            let content = Content::from_cbor(&decrypted.plaintext)
                .unwrap_or_else(|err| panic!("{msg_id} holds no content: {err:?}"));
            match content.body.clone() {
                Some(Body::Receipt(receipt)) => Some(Got {
                    msg_id,
                    sender: decrypted.sender,
                    content,
                    receipt,
                }),
                _ => None,
            }
        })
        .collect()
}

/// Ada's agents and second machine with Bob's played device: A, B and C.
pub struct Roles {
    pub senders: Senders,
}

impl Roles {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, as [`Senders::new`]
    /// sets them up.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Roles {
            senders: Senders::new(ids, logins),
        }
    }

    /// A's machine: Ada's second one.
    pub fn a(&self) -> &Machine {
        &self.senders.laptop.machine
    }

    /// B's machine: Ada's first one.
    pub fn b(&self) -> &Machine {
        &self.senders.agents.talk.ada.machine
    }

    /// B's device.
    pub fn b_device(&self) -> DeviceId {
        self.senders.agents.talk.ada.device
    }

    /// The channel.
    pub fn channel(&self) -> ChannelId {
        self.senders.agents.talk.channel
    }

    /// A revision B's device signs takes Bob, C's person, out of the channel.
    pub fn c_leaves(&self) {
        let talk = &self.senders.agents.talk;
        let leave = vec![(talk.channel, &talk.played[0].person)];
        let previous = latest(talk.cloud(), &talk.ada.token, talk.space.id);
        let change = Change {
            leave,
            ..Change::default()
        };
        revise(talk.cloud(), &previous, &talk.ada, &change);
    }

    /// claude's address in the short form `--to` takes: `<ada>/claude-…`.
    pub fn claude(&self) -> String {
        format!("{}/{CLAUDE}", self.senders.ada())
    }

    /// The full address of Ada's agent `agent` in `acme`.
    pub fn full(&self, agent: &str) -> String {
        format!("{ACME}/{}/{agent}@{HOST}", self.senders.ada())
    }

    /// Runs `skald --agent <A's agent> send --json <addressee> --channel <channel> <text>` on A's
    /// machine in its `~/work/acme`; fails unless it succeeds, reporting one `msg_id`, and the
    /// front saw that message sent in the channel. Gives its `msg_id`, as the run reports it: B's
    /// device may send before the run is over.
    #[track_caller]
    pub fn a_sends(&self, addressee: &[&str], text: &str) -> MessageId {
        let talk = &self.senders.agents.talk;
        let channel = talk.channel.to_string();
        let from = talk.front.sent().len();
        let mut args = vec![AGENT, LAPTOP_AGENT, SEND, JSON_FLAG];
        args.extend_from_slice(addressee);
        args.extend([CHANNEL, channel.as_str(), text]);
        let sent = (self.a())
            .start_in(&self.senders.laptop_folder, &args)
            .finish();
        assert!(sent.success, "A's agent: {sent:?}");
        let ids: Vec<MessageId> = (json_objects(&sent.stdout).iter())
            .filter_map(|object| object["msg_id"].as_str()?.parse().ok())
            .collect();
        let msg_id = match ids.as_slice() {
            [id, ..] if ids.iter().all(|one| one == id) => *id,
            _ => panic!("not one msg_id: {sent:?}"),
        };
        let seen = messages_sent(&talk.front, talk.channel, from);
        assert!(
            (seen.iter()).any(|one| one.envelope.envelope().header.msg_id == msg_id),
            "{msg_id} not sent: {seen:?}"
        );
        msg_id
    }

    /// As [`Roles::a_sends`], `--to` claude.
    #[track_caller]
    pub fn a_sends_to_claude(&self, text: &str) -> MessageId {
        let claude = self.claude();
        self.a_sends(&[TO, &claude], text)
    }

    /// Runs `skald --agent <A's agent> recv <flags>` on A's machine in its `~/work/acme`; fails
    /// unless it succeeds.
    #[track_caller]
    pub fn a_agent_reads(&self, flags: &[&str]) -> Finished {
        let mut args = vec![AGENT, LAPTOP_AGENT, RECV];
        args.extend_from_slice(flags);
        let read = (self.a())
            .start_in(&self.senders.laptop_folder, &args)
            .finish();
        assert!(read.success, "A's agent: {read:?}");
        read
    }

    /// As [`Roles::a_agent_reads`], with `--json`.
    #[track_caller]
    pub fn a_agent_reads_json(&self) -> Finished {
        self.a_agent_reads(&[JSON_FLAG])
    }

    /// Starts `skald --agent claude recv <flags>` on B's machine in its `~/work/acme`.
    pub fn claude_starts(&self, flags: &[&str]) -> Run {
        let mut args = vec![AGENT, CLAUDE, RECV];
        args.extend_from_slice(flags);
        self.b().start_in(&self.senders.agents.folder, &args)
    }

    /// As [`Roles::claude_starts`], each request with the timeout the tests give a run that
    /// retries.
    pub fn claude_starts_retrying(&self) -> Run {
        let args = [AGENT, CLAUDE, RECV];
        let mut command = self.b().command_in(&self.senders.agents.folder, &args);
        command
            .env(TIMEOUT_SETTING, TIMEOUT.as_secs().to_string())
            .stdin(Stdio::null());
        Run::spawn(&mut command)
    }

    /// Runs `skald --agent claude recv`, with `--wait` if `wait`, on B's machine; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn claude_reads(&self, wait: bool) -> Finished {
        self.senders.claude_reads(wait)
    }

    /// C takes in its mailbox every half second, for up to [`WAIT`], until it has read a receipt
    /// in the channel that lists `msg_id`; fails if none comes. Gives every receipt it read there.
    #[track_caller]
    pub fn c_gets_receipt_of(&mut self, msg_id: MessageId) -> Vec<Got> {
        let channel = self.channel();
        let talk = &mut self.senders.agents.talk;
        let deadline = Instant::now() + WAIT;
        loop {
            talk.played[0].take_in(&talk.world.cloud);
            let got = receipts_in(&talk.played[0], channel);
            if got.iter().any(|one| one.lists(msg_id)) {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "no receipt of {msg_id} read: {got:?}"
            );
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// C takes in its mailbox; gives every receipt it read in the channel so far.
    pub fn c_receipts(&mut self) -> Vec<Got> {
        let channel = self.channel();
        let talk = &mut self.senders.agents.talk;
        talk.played[0].take_in(&talk.world.cloud);
        receipts_in(&talk.played[0], channel)
    }
}

/// Fails unless `shown` holds `text` on one line.
#[track_caller]
pub fn assert_shows_once(shown: &str, text: &str) {
    let lines = shown.lines().filter(|line| line.contains(text)).count();
    assert_eq!(lines, 1, "{text:?} not shown once: {shown}");
}

/// The flags of a waiting `skald recv`.
pub fn waiting() -> [&'static str; 1] {
    [WAIT_FLAG]
}
