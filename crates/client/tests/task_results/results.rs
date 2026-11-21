//! What the tests share: the claims of the tests of claims, with Bob's device claiming and
//! completing through the server's requests and writing the results the tests craft in the
//! channel's group; what a run shows of a result, of a dropped message and of a `read` receipt.

use std::num::NonZeroU64;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_proto::{
    Body, Cbor as _, ChannelId, ClaimId, Content, DeviceId, Envelope, MessageId, Outcome,
    RawEnvelope, ReceiptStatus, SpaceId, TaskId, TaskResult, Unknown,
};

use crate::agents::CLAUDE;
use crate::claiming::{Claims, address};
use crate::cloud::WAIT;
use crate::fixtures::CHANNEL;
use crate::lines::{json_objects, refers_to};
use crate::machine::{Finished, Machine};
use crate::played::mls_envelope;
use crate::queuing::{default_queue, enqueued, references, refers_to_task, tasks};
use crate::states::{self, Held};
use crate::talk::stored;

/// How a result's line names its outcome.
pub fn outcome_mark(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "(task done)",
        Outcome::Failed => "(task failed)",
    }
}

/// The word of a dropped message's warning.
pub const DROPPED: &str = "dropped";

/// A claim as a result names it: its `claim_id` and generation.
pub type Named = (ClaimId, u64);

/// The claims of the tests of claims, and the counter of Bob's device's last line in the channel.
pub struct Results {
    pub claims: Claims,
    bob_said: u64,
}

impl Results {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        Results {
            claims: Claims::new(ids, logins),
            bob_said: 0,
        }
    }

    /// The full address of Bob's agent his device writes results as.
    pub fn bob_agent(&self) -> String {
        self.claims.senders.bob_agent()
    }

    /// As `agent`, or the person with none, on `machine` in `dir`, `skald enqueue --channel
    /// <channel> <args>`, which must succeed, in `at`'s channel; gives the task it put in the
    /// channel's queue `default`.
    #[track_caller]
    pub fn enqueues(
        &self,
        (machine, dir, agent): (&Machine, &Path, Option<&str>),
        at: (SpaceId, ChannelId),
        args: &[&str],
    ) -> TaskId {
        let (cloud, ada) = (self.claims.cloud(), self.claims.ada());
        let queue = default_queue(cloud, ada, at);
        let ids = || -> Vec<TaskId> {
            (tasks(cloud, ada, at, queue).into_iter())
                .map(|stored| stored.task.task)
                .collect()
        };
        let before = ids();
        let channel = at.1.to_string();
        let mut all = vec![CHANNEL, channel.as_str()];
        all.extend_from_slice(args);
        enqueued(machine, (dir, agent), &all);
        let new: Vec<TaskId> = (ids().into_iter())
            .filter(|task| !before.contains(task))
            .collect();
        let [task] = new.as_slice() else {
            panic!("not one task enqueued: {new:?}");
        };
        *task
    }

    /// As claude on Ada's first machine, [`Results::enqueues`] in the talk's channel.
    #[track_caller]
    pub fn claude_enqueues(&self, args: &[&str]) -> TaskId {
        let agents = &self.claims.senders.agents;
        let at = (
            &agents.talk.ada.machine,
            agents.folder.as_path(),
            Some(CLAUDE),
        );
        self.enqueues(at, self.claims.at, args)
    }

    /// Bob's device claims a task of the queue `default`; fails unless it gets one.
    #[track_caller]
    pub fn bob_claims(&self) -> Held {
        let (cloud, at, queue) = (self.claims.cloud(), self.claims.at, self.claims.queue);
        states::claims(cloud, &self.claims.bob().person, at, queue)
    }

    /// Bob's device completes `held`; fails unless the server takes it.
    #[track_caller]
    pub fn bob_completes(&self, held: &Held) {
        let (cloud, bob) = (self.claims.cloud(), &self.claims.bob().person);
        states::taken(cloud, bob, held, ("complete", json!({})));
    }

    /// Bob's device writes, in the channel's group as its next line there, the content `content`
    /// makes of the envelope's `msg_id` and that line's counter; fails unless the server stores
    /// it. Gives its `msg_id`.
    #[track_caller]
    pub fn bob_writes(
        &mut self,
        content: impl FnOnce(MessageId, NonZeroU64) -> Content,
    ) -> MessageId {
        self.bob_said += 1;
        let counter = NonZeroU64::new(self.bob_said).expect("a counter");
        let at = self.claims.at;
        let talk = &mut self.claims.senders.agents.talk;
        talk.played[0].take_in(&talk.world.cloud);
        let header = mls_envelope(at, &[], &[]).envelope().header.clone();
        let msg_id = header.msg_id;
        let content = content(msg_id, counter);
        let payload = (talk.played[0].mls)
            .encrypt(at.1, &content.to_cbor())
            .unwrap_or_else(|err| panic!("encrypt in the channel's group: {err:?}"));
        let envelope =
            RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope");
        stored(&talk.world.cloud, &talk.played[0], &envelope);
        msg_id
    }

    /// Bob's device writes a result of `task` from Bob's agent to `to`, naming the claim `named`,
    /// with `outcome` and `text`, as [`Results::bob_writes`] does. Gives its `msg_id`.
    #[track_caller]
    pub fn bob_results(
        &mut self,
        (task, named): (TaskId, Named),
        to: &str,
        (outcome, text): (Outcome, &str),
    ) -> MessageId {
        let from = address(&self.bob_agent());
        let to = address(to);
        self.bob_writes(|msg_id, counter| {
            result(task, named, (outcome, text), msg_id).content((from, to), counter, 1)
        })
    }

    /// As [`Results::bob_results`], to claude, naming `held`.
    #[track_caller]
    pub fn bob_results_to_claude(
        &mut self,
        held: &Held,
        (outcome, text): (Outcome, &str),
    ) -> MessageId {
        let claude = self.claims.claude();
        self.bob_results((held.task, named(held)), &claude, (outcome, text))
    }

    /// Runs `skald --agent claude recv` on Ada's first machine; fails unless it succeeds.
    #[track_caller]
    pub fn claude_reads(&self) -> Finished {
        self.claims.senders.claude_reads(false)
    }

    /// As [`Results::claude_reads`], with `--json`.
    #[track_caller]
    pub fn claude_reads_json(&self) -> Finished {
        let agents = &self.claims.senders.agents;
        let args = [
            crate::world::AGENT,
            CLAUDE,
            crate::kit::RECV,
            crate::lines::JSON_FLAG,
        ];
        let read = agents
            .talk
            .ada
            .machine
            .start_in(&agents.folder, &args)
            .finish();
        assert!(read.success, "claude: {read:?}");
        read
    }

    /// Waits until Bob's device read `count` results of `task` in the channel, taking in again
    /// every half second for up to [`WAIT`]; fails if they do not come.
    #[track_caller]
    pub fn bob_reads_results(&mut self, task: TaskId, count: usize) {
        let deadline = Instant::now() + WAIT;
        while self.claims.results(task).len() < count {
            assert!(
                Instant::now() < deadline,
                "not {count} results of {task} read"
            );
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// The `read` receipts of `msg_id` Bob's device reads in the channel, each with its sending
    /// device and content, once it reads one; takes in again every half second for up to
    /// [`WAIT`]. Fails if none comes.
    #[track_caller]
    pub fn read_receipts_of(&mut self, msg_id: MessageId) -> Vec<(DeviceId, Content)> {
        let deadline = Instant::now() + WAIT;
        loop {
            let talk = &mut self.claims.senders.agents.talk;
            talk.played[0].take_in(&talk.world.cloud);
            let read: Vec<_> = (talk.played[0].read_in(talk.channel).into_iter())
                .filter_map(|(_, decrypted)| {
                    let content = Content::from_cbor(&decrypted.plaintext).ok()?;
                    let Some(Body::Receipt(receipt)) = &content.body else {
                        return None;
                    };
                    (receipt.status == ReceiptStatus::Read && receipt.messages.contains(&msg_id))
                        .then_some((decrypted.sender, content))
                })
                .collect();
            if !read.is_empty() {
                return read;
            }
            assert!(Instant::now() < deadline, "no read of {msg_id}");
            thread::sleep(Duration::from_millis(500));
        }
    }
}

/// The claim `held` names, as a result names it.
#[track_caller]
pub fn named(held: &Held) -> Named {
    let claim_id = (held.claim_id.parse()).unwrap_or_else(|_| panic!("no claim ID in {held:?}"));
    (claim_id, held.generation)
}

/// A result of `task` naming the claim `named`, with `outcome`, whose response of the role
/// `ROLE_AGENT` holds `text` and has the `messageId` `msg_id`.
pub fn result(
    task: TaskId,
    (claim_id, generation): Named,
    (outcome, text): (Outcome, &str),
    msg_id: MessageId,
) -> TaskResult {
    let response = json!({
        "messageId": msg_id.to_string(),
        "role": "ROLE_AGENT",
        "parts": [{ "text": text }],
    });
    TaskResult {
        task,
        claim_id,
        generation,
        outcome,
        response: response.to_string(),
        unknown: Unknown::default(),
    }
}

/// Fails unless `shown`, an agent's output, has one line holding `text`, and it names `task` by
/// its reference and `outcome` as [`outcome_mark`] does, before its text. Gives the line.
#[track_caller]
pub fn assert_result_line(shown: &str, task: TaskId, (outcome, text): (Outcome, &str)) -> String {
    let lines: Vec<&str> = shown.lines().filter(|line| line.contains(text)).collect();
    let [line] = lines.as_slice() else {
        panic!("not one line holds {text:?}: {shown}");
    };
    let (head, _) = line.split_once(text).expect("the text");
    assert!(
        head.ends_with(&format!(" {}: ", outcome_mark(outcome))),
        "not a {outcome:?} result line: {line:?}"
    );
    assert!(
        refers_to_task(head, task),
        "not the task's reference: {line:?}"
    );
    (*line).to_owned()
}

/// Fails unless `finished` says, on its standard error, that it dropped the message `msg_id`,
/// naming it by its `msg_id` or reference, and shows nothing of `text`.
#[track_caller]
pub fn assert_dropped(finished: &Finished, msg_id: MessageId, text: &str) {
    let id = msg_id.to_string();
    let names = |line: &str| {
        line.contains(&id) || (references(line).iter()).any(|short| refers_to(short, msg_id))
    };
    let warned =
        (finished.stderr.lines()).any(|line| line.to_lowercase().contains(DROPPED) && names(line));
    assert!(warned, "not said that {msg_id} was dropped: {finished:?}");
    assert!(
        !finished.shown().contains(text),
        "{text:?} shown: {finished:?}"
    );
}

/// The JSON objects of the type `result` of `stdout`, the output of `skald recv --json`.
pub fn result_objects(stdout: &str) -> Vec<Value> {
    (json_objects(stdout).into_iter())
        .filter(|object| object["type"] == "result")
        .collect()
}
