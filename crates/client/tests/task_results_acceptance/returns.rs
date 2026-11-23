//! What the tests share: the scene of the tests of claiming tasks, tasks claude enqueues, claims
//! Bob's device holds and the results it writes and sends, and what the CLI shows of results and
//! of the messages it drops.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::path::Path;

use serde_json::{Value, json};
use skald_proto::{
    Cbor as _, ChannelId, ClaimId, Content, DeviceId, Envelope, Header, MessageId, Outcome,
    QueueId, RawEnvelope, SpaceId, TaskId, TaskResult, Unknown,
};

use crate::agents::{CLAUDE, run_as_in};
use crate::claiming::{Claims, address};
use crate::fixtures::CHANNEL;
use crate::kit::{RECV, received};
use crate::lines::{JSON_FLAG, json_objects, refers_to};
use crate::machine::{Finished, Machine};
use crate::played::mls_envelope;
use crate::queuing::{default_queue, enqueued, refers_to_task, run_in, tasks};
use crate::senders::holds_word;
use crate::states::{self, Held};
use crate::talk::stored;
use crate::world::warnings;

/// The outcomes as results show them.
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";

/// The request that completes a claim.
pub const COMPLETE_ACT: &str = "complete";

/// A result a test writes: its `task-result` body and its content's `from` and `to`, full
/// addresses.
#[derive(Debug, Clone)]
pub struct Answer {
    pub task: TaskId,
    pub claim_id: ClaimId,
    pub generation: u64,
    pub outcome: Outcome,
    pub from: String,
    pub to: String,
    /// The one text part of its `response`.
    pub text: String,
}

impl Answer {
    /// A result of the claim `held` with `outcome` and `text`, from `from` to `to`.
    #[track_caller]
    pub fn of(held: &Held, outcome: Outcome, (from, to): (&str, &str), text: &str) -> Self {
        Answer {
            task: held.task,
            claim_id: (held.claim_id.parse()).unwrap_or_else(|_| panic!("no claim ID in {held:?}")),
            generation: held.generation,
            outcome,
            from: from.to_owned(),
            to: to.to_owned(),
            text: text.to_owned(),
        }
    }

    /// Its content in the envelope `msg_id`, with `counter` and `hops` 1, as "Results" says: the
    /// response's `messageId` is the envelope's `msg_id`, its role `ROLE_AGENT`.
    pub fn content(&self, msg_id: MessageId, counter: u64) -> Content {
        let response = json!({
            "messageId": msg_id.to_string(),
            "role": "ROLE_AGENT",
            "parts": [{ "text": self.text }],
        });
        TaskResult {
            task: self.task,
            claim_id: self.claim_id,
            generation: self.generation,
            outcome: self.outcome,
            response: response.to_string(),
            unknown: Unknown::default(),
        }
        .content(
            (address(&self.from), address(&self.to)),
            NonZeroU64::new(counter).expect("a counter of at least 1"),
            1,
        )
    }
}

/// The scene of the tests of claiming tasks, with the counter of Bob's last message in each
/// channel.
pub struct Returns {
    pub claims: Claims,
    said: HashMap<ChannelId, u64>,
}

impl Returns {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones, as [`Claims::new`]
    /// sets them up.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Returns {
            claims: Claims::new(ids, logins),
            said: HashMap::new(),
        }
    }

    /// The talk's channel, and `acme`'s default channel.
    pub fn channel(&self) -> ChannelId {
        self.claims.at.1
    }

    pub fn default_channel(&self) -> ChannelId {
        self.claims.talk().space.default
    }

    /// `channel` with its space.
    pub fn at(&self, channel: ChannelId) -> (SpaceId, ChannelId) {
        (self.claims.at.0, channel)
    }

    /// The queue `default` of `channel`.
    #[track_caller]
    pub fn queue_of(&self, channel: ChannelId) -> QueueId {
        default_queue(self.claims.cloud(), self.claims.ada(), self.at(channel))
    }

    /// The full address of Bob's agent `claude-q3x6np`.
    pub fn bob_agent(&self) -> String {
        self.claims.senders.bob_agent()
    }

    /// Bob's device.
    pub fn bob_device(&self) -> DeviceId {
        self.claims.bob().device()
    }

    /// As `agent`, or the person with none, on `machine` in `dir`, `skald enqueue --channel
    /// <channel> <args>`, which must succeed; gives the one task it added to `channel`'s queue
    /// `default`.
    #[track_caller]
    pub fn enqueues(
        &self,
        (machine, dir, agent): (&Machine, &Path, Option<&str>),
        channel: ChannelId,
        args: &[&str],
    ) -> TaskId {
        let (cloud, ada) = (self.claims.cloud(), self.claims.ada());
        let (at, queue) = (self.at(channel), self.queue_of(channel));
        let ids = || -> Vec<TaskId> {
            (tasks(cloud, ada, at, queue).iter())
                .map(|stored| stored.task.task)
                .collect()
        };
        let before = ids();
        let named = channel.to_string();
        let mut all = vec![CHANNEL, named.as_str()];
        all.extend_from_slice(args);
        enqueued(machine, (dir, agent), &all);
        let added: Vec<TaskId> = (ids().into_iter())
            .filter(|task| !before.contains(task))
            .collect();
        let [one] = added.as_slice() else {
            panic!("not one task enqueued in {channel}: {added:?}");
        };
        *one
    }

    /// As claude on Ada's first machine, [`Returns::enqueues`] of `<extra> <text>`.
    #[track_caller]
    pub fn claude_enqueues_in(&self, channel: ChannelId, extra: &[&str], text: &str) -> TaskId {
        let mut args = extra.to_vec();
        args.push(text);
        let claude = (
            &self.claims.ada().machine,
            self.claims.senders.agents.folder.as_path(),
            Some(CLAUDE),
        );
        self.enqueues(claude, channel, &args)
    }

    /// As [`Returns::claude_enqueues_in`], in the talk's channel.
    #[track_caller]
    pub fn claude_enqueues(&self, text: &str) -> TaskId {
        self.claude_enqueues_in(self.channel(), &[], text)
    }

    /// Bob's device claims a task of `channel`'s queue `default`, stating `peer`, as the tests of
    /// task states do; fails unless it gets one.
    #[track_caller]
    pub fn bob_claims_in(&self, channel: ChannelId) -> Held {
        let queue = self.queue_of(channel);
        let bob = &self.claims.bob().person;
        states::claims(self.claims.cloud(), bob, self.at(channel), queue)
    }

    /// As [`Returns::bob_claims_in`], in the talk's channel.
    #[track_caller]
    pub fn bob_claims(&self) -> Held {
        self.bob_claims_in(self.channel())
    }

    /// Bob's device completes `held`; fails unless the server takes it.
    #[track_caller]
    pub fn bob_completes(&self, held: &Held) {
        let bob = &self.claims.bob().person;
        states::taken(self.claims.cloud(), bob, held, (COMPLETE_ACT, json!({})));
    }

    /// Bob's device fails `held`, with retry if `retry`; fails unless the server takes it.
    #[track_caller]
    pub fn bob_fails(&self, held: &Held, retry: bool) {
        let bob = &self.claims.bob().person;
        states::fails(self.claims.cloud(), bob, held, retry);
    }

    /// Bob's device claims a task of `channel`'s queue `default` and completes it, as
    /// [`Returns::bob_claims_in`] and [`Returns::bob_completes`] do. Gives a `done` result of that
    /// claim with `text`, from Bob's agent `claude-q3x6np` to claude.
    #[track_caller]
    pub fn bob_runs_in(&self, channel: ChannelId, text: &str) -> Answer {
        let held = self.bob_claims_in(channel);
        self.bob_completes(&held);
        let (bob_agent, claude) = (self.bob_agent(), self.claims.claude());
        Answer::of(&held, Outcome::Done, (&bob_agent, &claude), text)
    }

    /// As [`Returns::bob_runs_in`], in the talk's channel.
    #[track_caller]
    pub fn bob_runs(&self, text: &str) -> Answer {
        self.bob_runs_in(self.channel(), text)
    }

    /// The counter of Bob's next message in `channel`.
    pub fn next_counter(&mut self, channel: ChannelId) -> u64 {
        let said = self.said.entry(channel).or_insert(0);
        *said += 1;
        *said
    }

    /// Bob's device sends `answer` in `channel`, under `mls` to the whole channel, with its next
    /// counter there; fails unless the server stores it. Gives its `msg_id`.
    #[track_caller]
    pub fn bob_answers_in(&mut self, channel: ChannelId, answer: &Answer) -> MessageId {
        let header = self.header_in(channel);
        let counter = self.next_counter(channel);
        let content = answer.content(header.msg_id, counter);
        self.bob_sends(header, &content)
    }

    /// As [`Returns::bob_answers_in`], in the talk's channel.
    #[track_caller]
    pub fn bob_answers(&mut self, answer: &Answer) -> MessageId {
        self.bob_answers_in(self.channel(), answer)
    }

    /// A new header of an `mls` envelope of `channel` to the whole channel.
    pub fn header_in(&self, channel: ChannelId) -> Header {
        mls_envelope(self.at(channel), &[], &[])
            .envelope()
            .header
            .clone()
    }

    /// Bob's device takes in its mailbox, then sends `content` in the envelope with `header`,
    /// encrypted in its channel's group; fails unless the server stores it. Gives its `msg_id`.
    #[track_caller]
    pub fn bob_sends(&mut self, header: Header, content: &Content) -> MessageId {
        let talk = &mut self.claims.senders.agents.talk;
        let cloud = &talk.world.cloud;
        let bob = &mut talk.played[0];
        bob.take_in(cloud);
        let (channel, msg_id) = (header.channel, header.msg_id);
        let payload = (bob.mls)
            .encrypt(channel, &content.to_cbor())
            .unwrap_or_else(|err| panic!("encrypt in the group of {channel}: {err:?}"));
        let envelope =
            RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope");
        stored(cloud, bob, &envelope);
        msg_id
    }

    /// Runs `skald --agent claude recv` on Ada's first machine in `~/work/acme`; fails unless it
    /// succeeds.
    #[track_caller]
    pub fn claude_reads(&self) -> Finished {
        let agents = &self.claims.senders.agents;
        run_as_in(&agents.talk.ada.machine, &agents.folder, CLAUDE)
    }

    /// As [`Returns::claude_reads`], with `--json`.
    #[track_caller]
    pub fn claude_reads_json(&self) -> Finished {
        let agents = &self.claims.senders.agents;
        let at = (agents.folder.as_path(), Some(CLAUDE));
        let read = run_in(&agents.talk.ada.machine, at, RECV, &[JSON_FLAG]);
        assert!(read.success, "claude's recv --json: {read:?}");
        read
    }

    /// Runs the person's `skald recv` on Ada's first machine; fails unless it succeeds.
    #[track_caller]
    pub fn ada_reads(&self) -> Finished {
        received(&self.claims.ada().machine)
    }

    /// claude's next `skald recv`, then the person's on Ada's first machine: fails if either shows
    /// a result of `task`, or unless the person's warns of one from Bob's device, naming that
    /// device, the task's ID or its reference. `what` names the case.
    #[track_caller]
    pub fn assert_dropped(&self, task: TaskId, what: &str) {
        let claude = self.claude_reads();
        assert_no_result(&claude.shown(), task, &format!("{what}: claude"));
        let person = self.ada_reads();
        assert_no_result(&person.shown(), task, &format!("{what}: the person"));
        assert!(
            warns(&person, self.bob_device(), task),
            "{what}: no warning of the dropped result: {person:?}"
        );
    }

    /// Bob's device sends `forged`, which claude's next run must drop as [`Returns::assert_dropped`]
    /// says, then `right`, a `done` result of `task` that claude's next run must hand over as
    /// [`Returns::assert_claude_gets_bob_s`] says. `what` names the case.
    #[track_caller]
    pub fn assert_drops_then_takes(
        &mut self,
        task: TaskId,
        (forged, right): (&Answer, &Answer),
        what: &str,
    ) {
        self.bob_answers(forged);
        self.assert_dropped(task, what);
        self.bob_answers(right);
        self.assert_claude_gets_bob_s(task, DONE);
    }

    /// Fails unless claude's next `skald recv` hands over one result of `task` with `outcome`,
    /// from Bob's agent at the level `peer`. Gives what it showed.
    #[track_caller]
    pub fn assert_claude_gets_bob_s(&self, task: TaskId, outcome: &str) -> String {
        let shown = self.claude_reads().shown();
        assert_one_result(&shown, task, outcome, (crate::senders::BOB_AGENT, PEER));
        shown
    }
}

/// The trust levels as the CLI shows them.
pub const OWN: &str = "own";
pub const PEER: &str = "peer";
pub const SERVICE: &str = "service";

/// The lines of `shown` that hand over a result of `task` with `outcome`: they hold a reference to
/// the task and `task <outcome>`, in any case.
pub fn result_lines(shown: &str, task: TaskId, outcome: &str) -> Vec<String> {
    let said = format!("task {outcome}");
    (shown.lines())
        .filter(|line| refers_to_task(line, task) && line.to_lowercase().contains(&said))
        .map(str::to_owned)
        .collect()
}

/// Fails unless `shown` hands over one result of `task` with `outcome`, in a line that names the
/// claiming agent `agent` and holds the level `level` as a word. Gives that line.
#[track_caller]
pub fn assert_one_result(
    shown: &str,
    task: TaskId,
    outcome: &str,
    (agent, level): (&str, &str),
) -> String {
    let lines = result_lines(shown, task, outcome);
    let [line] = lines.as_slice() else {
        panic!("not one {outcome} result of {task}: {shown}");
    };
    assert!(line.contains(agent), "{line:?} does not name {agent}");
    assert!(holds_word(line, level), "{line:?} does not say {level}");
    line.clone()
}

/// Fails if `shown` hands over any result of `task`. `what` names the case.
#[track_caller]
pub fn assert_no_result(shown: &str, task: TaskId, what: &str) {
    for outcome in [DONE, FAILED] {
        let lines = result_lines(shown, task, outcome);
        assert!(lines.is_empty(), "{what}: a result handed over: {lines:?}");
    }
}

/// Whether a warning line of `finished` names `device`, the task `task`'s ID or its reference.
pub fn warns(finished: &Finished, device: DeviceId, task: TaskId) -> bool {
    let (device, id) = (device.to_string(), task.to_string());
    (warnings(finished).iter())
        .any(|line| line.contains(&device) || line.contains(&id) || refers_to_task(line, task))
}

/// Whether `value` or a value nested in it is a text that holds `part`.
pub fn holds_part(value: &Value, part: &str) -> bool {
    match value {
        Value::String(held) => held.contains(part),
        Value::Array(items) => items.iter().any(|item| holds_part(item, part)),
        Value::Object(fields) => fields.values().any(|field| holds_part(field, part)),
        _ => false,
    }
}

/// Whether `value` or a value nested in it names the task `task`: its ID, or a reference to it,
/// with or without its `#`.
pub fn names_task(value: &Value, task: TaskId) -> bool {
    let as_message = MessageId::from_bytes(*task.as_bytes());
    match value {
        Value::String(held) => {
            let short = held.trim_start_matches('#');
            *held == task.to_string()
                || (short.chars().all(|c| c.is_ascii_alphanumeric())
                    && refers_to(short, as_message))
        }
        Value::Array(items) => items.iter().any(|item| names_task(item, task)),
        Value::Object(fields) => fields.values().any(|field| names_task(field, task)),
        _ => false,
    }
}

/// The JSON objects of `stdout` that are results of `task` with `outcome`: they name the task and
/// have a value that is `outcome`.
pub fn result_objects(stdout: &str, task: TaskId, outcome: &str) -> Vec<Value> {
    (json_objects(stdout).into_iter())
        .filter(|object| names_task(object, task))
        .filter(|object| crate::lines::holds_text(object, outcome))
        .collect()
}
