//! What the tests share: Ada's two machines and Bob's played device in a channel, the claim
//! commands as an agent or the person runs them, tasks the tests write and upload, what the server
//! gives of a task, a compromised server's changes to its copy of a task, and the results a device
//! reads.

use std::num::NonZeroU64;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use skald_crypto::{SigningKey, seal_task, sign_task};
use skald_proto::{
    Address, Cbor as _, ChannelId, Claim, ClaimId, Content, DeviceId, MessageId, Outcome, QueueId,
    QueueMode, QueuedTask, Sched, SpaceId, Task, TaskBody, TaskId, TaskPlaintext, TaskResult,
    Unknown,
};

use crate::agents::{CLAUDE, run_as_in};
use crate::cloud::{Cloud, WAIT};
use crate::fixtures::{CHANNEL, hex};
use crate::machine::{Finished, Machine};
use crate::played::Played;
use crate::queuing::{
    default_queue, enqueued, proved, proved_with, queues_path, refers_to_task, run_in, tasks_path,
    the_one_task,
};
use crate::senders::{LAPTOP_AGENT, Senders};
use crate::talk::Talk;
use crate::world::{Person, now};

/// The claim commands.
pub const CLAIM: &str = "claim";
pub const EXTEND: &str = "extend";
pub const COMPLETE: &str = "complete";
pub const FAIL: &str = "fail";

/// Their options.
pub const QUEUE_FLAG: &str = "--queue";
pub const PROGRESS_FLAG: &str = "--progress";
pub const RETRY_FLAG: &str = "--retry";
pub const JSON_FLAG: &str = "--json";

/// The states, progress and reasons of a task as the server gives them.
pub const QUEUED: &str = "queued";
pub const CLAIMED: &str = "claimed";
pub const DONE: &str = "done";
pub const DEAD: &str = "dead";
pub const CANCELLED: &str = "cancelled";
pub const STARTED: &str = "started";
pub const AWAITING_APPROVAL: &str = "awaiting-approval";
pub const FAILED: &str = "failed";

/// What a claim with no task says, and what a command on a lost claim says.
pub const NO_TASK: &str = "no task";
pub const LOST: &str = "lost";

/// The key of the `task-result` body in content.
const TASK_RESULT: i128 = 12;

/// Ada's two machines, the second's agent `codex-v5d8tm` having run once in its `~/work/acme`,
/// and Bob's played device, in the talk's channel, with its queue `default`.
pub struct Claims {
    pub senders: Senders,
    pub at: (SpaceId, ChannelId),
    /// The channel's ID, as a command line names it.
    pub channel: String,
    /// The channel's queue `default`.
    pub queue: QueueId,
}

impl Claims {
    /// Ada, GitHub user `ids[0]` with `logins[0]`, and Bob, the next ones.
    #[track_caller]
    pub fn new(ids: [u64; 2], logins: [&str; 2]) -> Self {
        let mut senders = Senders::new(ids, logins);
        run_as_in(
            &senders.laptop.machine,
            &senders.laptop_folder,
            LAPTOP_AGENT,
        );
        let talk = &mut senders.agents.talk;
        talk.played[0].take_in(&talk.world.cloud);
        let at = talk.at();
        let queue = default_queue(&talk.world.cloud, &talk.ada, at);
        Claims {
            senders,
            at,
            channel: at.1.to_string(),
            queue,
        }
    }

    pub fn talk(&self) -> &Talk {
        &self.senders.agents.talk
    }

    pub fn cloud(&self) -> &Cloud {
        self.talk().cloud()
    }

    /// Ada's first machine.
    pub fn ada(&self) -> &Person {
        &self.talk().ada
    }

    /// Ada's second machine.
    pub fn laptop(&self) -> &Person {
        &self.senders.laptop
    }

    /// Bob's played device.
    pub fn bob(&self) -> &Played {
        &self.talk().played[0]
    }

    /// The full address of codex on the second machine.
    pub fn codex(&self) -> String {
        self.senders.agents.address(LAPTOP_AGENT)
    }

    /// The full address of claude on the first machine.
    pub fn claude(&self) -> String {
        self.senders.agents.address(CLAUDE)
    }

    /// The full address of an agent `agent` of Ada's.
    pub fn ada_agent(&self, agent: &str) -> String {
        self.senders.agents.address(agent)
    }

    /// Ada's person address.
    pub fn person(&self) -> String {
        self.talk().address(self.ada())
    }

    /// Bob's person address.
    pub fn bob_person(&self) -> String {
        self.talk().address(&self.bob().person)
    }

    /// Runs `skald --agent codex-v5d8tm <command> <args>` on the second machine in its
    /// `~/work/acme`.
    pub fn codex_runs(&self, command: &str, args: &[&str]) -> Finished {
        let at = (self.senders.laptop_folder.as_path(), Some(LAPTOP_AGENT));
        run_in(&self.laptop().machine, at, command, args)
    }

    /// Runs `skald <command> <args>` as the person on the second machine in its `~/work/acme`.
    pub fn person_runs(&self, command: &str, args: &[&str]) -> Finished {
        let at = (self.senders.laptop_folder.as_path(), None);
        run_in(&self.laptop().machine, at, command, args)
    }

    /// Runs `skald --agent codex-v5d8tm claim --channel <channel> <extra>` on the second machine.
    pub fn codex_claims(&self, extra: &[&str]) -> Finished {
        let mut args = vec![CHANNEL, self.channel.as_str()];
        args.extend_from_slice(extra);
        self.codex_runs(CLAIM, &args)
    }

    /// As [`Claims::codex_claims`]; fails unless it succeeds.
    #[track_caller]
    pub fn codex_claimed(&self, extra: &[&str]) -> Finished {
        let ran = self.codex_claims(extra);
        assert!(ran.success, "codex's claim: {ran:?}");
        ran
    }

    /// Runs codex's claims of [`Claims::codex_claims`] `times` times; gives what they showed
    /// together.
    pub fn codex_claims_times(&self, times: usize, extra: &[&str]) -> String {
        (0..times)
            .map(|_| self.codex_claims(extra).shown())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Runs `skald --agent codex-v5d8tm <command> <args>`; fails unless it succeeds.
    #[track_caller]
    pub fn codex_ran(&self, command: &str, args: &[&str]) -> Finished {
        let ran = self.codex_runs(command, args);
        assert!(ran.success, "codex's {command} {args:?}: {ran:?}");
        ran
    }

    /// As claude on the first machine, `skald enqueue --channel <channel> <text>`, which must
    /// succeed; gives the one task the queue `default` holds.
    #[track_caller]
    pub fn claude_enqueues(&self, text: &str) -> QueuedTask {
        let claude = (self.senders.agents.folder.as_path(), Some(CLAUDE));
        enqueued(&self.ada().machine, claude, &[CHANNEL, &self.channel, text]);
        the_one_task(self.cloud(), self.ada(), self.at, self.queue).task
    }

    /// A task `craft` makes, written as Ada's first device and uploaded with its proof; fails
    /// unless the server stores it.
    #[track_caller]
    pub fn ada_writes(&mut self, craft: &Craft) -> QueuedTask {
        let talk = &mut self.senders.agents.talk;
        talk.played[0].take_in(&talk.world.cloud);
        let signer = (&talk.ada.key, talk.ada.device);
        let record = sealed_by(&talk.played[0], self.at, signer, craft);
        uploads(&talk.world.cloud, &talk.ada, self.at, &record);
        record
    }

    /// A task `craft` makes, written as Bob's device and uploaded with its proof; fails unless the
    /// server stores it.
    #[track_caller]
    pub fn bob_writes(&mut self, craft: &Craft) -> QueuedTask {
        let talk = &mut self.senders.agents.talk;
        talk.played[0].take_in(&talk.world.cloud);
        let bob = &talk.played[0];
        let record = sealed_by(bob, self.at, (&bob.person.key, bob.device()), craft);
        uploads(&talk.world.cloud, &bob.person, self.at, &record);
        record
    }

    /// The task `task` of the queue `queue` as the server gives it to Ada's first device.
    #[track_caller]
    pub fn read_in(&self, queue: QueueId, task: TaskId) -> Read {
        read(self.cloud(), self.ada(), self.at, (queue, task))
    }

    /// As [`Claims::read_in`], in the queue `default`.
    #[track_caller]
    pub fn read(&self, task: TaskId) -> Read {
        self.read_in(self.queue, task)
    }

    /// The results of `task` Bob's device read in the channel, once it took in its mailbox.
    pub fn results(&mut self, task: TaskId) -> Vec<Got> {
        results_read(&mut self.senders.agents.talk, task)
    }

    /// As [`Claims::results`], once Bob's device read one; takes in again every half second for up
    /// to [`WAIT`]. Fails if none comes.
    #[track_caller]
    pub fn results_once_read(&mut self, task: TaskId) -> Vec<Got> {
        let deadline = Instant::now() + WAIT;
        loop {
            let read = self.results(task);
            if !read.is_empty() {
                return read;
            }
            assert!(Instant::now() < deadline, "no result of {task} read");
            thread::sleep(Duration::from_millis(500));
        }
    }

    /// Fails unless Bob's device reads one result of `task` in the channel, as [`assert_result`]
    /// checks it: from the second machine's device and codex to `to`, with `hops`, naming `held`,
    /// with `outcome` and `text`. Gives it.
    #[track_caller]
    pub fn assert_codex_result(
        &mut self,
        (task, held): (TaskId, &Held),
        (to, hops): (&str, u64),
        (outcome, text): (Outcome, &str),
    ) -> Got {
        let got = self.results_once_read(task);
        let [one] = got.as_slice() else {
            panic!("not one result: {got:?}");
        };
        let codex = self.codex();
        let expected = Expected {
            sender: self.laptop().device,
            from: &codex,
            to,
            hops,
            task,
            held,
            outcome,
            text,
        };
        assert_result(one, &expected);
        one.clone()
    }

    /// Codex's next `skald recv` runs, then Bob's device takes in; fails if it read a result of
    /// `task`.
    #[track_caller]
    pub fn assert_no_result(&mut self, task: TaskId, what: &str) {
        run_as_in(
            &self.senders.laptop.machine,
            &self.senders.laptop_folder,
            LAPTOP_AGENT,
        );
        let read = self.results(task);
        assert!(read.is_empty(), "{what}: a result sent: {read:?}");
    }
}

/// How a task the tests write differs from one as "Tasks" says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alter {
    Nothing,
    /// A byte of `ct` changed after sealing.
    Ct,
    /// A byte of the signature changed.
    Signature,
    /// The body names this device as `sender`, though another one signs it.
    Sender(DeviceId),
    /// The record's `sched` gets an `idempotency_key` the body's lacks.
    RecordSched,
}

/// A task the tests write.
#[derive(Debug, Clone)]
pub struct Craft {
    /// The one text part of its request.
    pub text: String,
    /// Its content's `from` and `to`.
    pub from: String,
    pub to: Option<String>,
    pub queue: QueueId,
    /// The mode its body states.
    pub mode: QueueMode,
    pub sched: Sched,
    pub hops: u64,
    pub counter: Option<u64>,
    pub alter: Alter,
}

impl Craft {
    /// A task of `text` from `from` in `queue`, `at-least-once`, as "Tasks" says.
    pub fn new(from: &str, queue: QueueId, text: &str) -> Self {
        Craft {
            text: text.to_owned(),
            from: from.to_owned(),
            to: None,
            queue,
            mode: QueueMode::AtLeastOnce,
            sched: Sched::default(),
            hops: 0,
            counter: None,
            alter: Alter::Nothing,
        }
    }

    /// As this one, changed as `alter` says.
    pub fn altered(mut self, alter: Alter) -> Self {
        self.alter = alter;
        self
    }
}

/// `text` as an address.
#[track_caller]
pub fn address(text: &str) -> Address {
    text.parse()
        .unwrap_or_else(|_| panic!("{text:?} is no address"))
}

/// The record of the task `craft` makes in `at`'s channel: signed by `signer`, the signing key and
/// device ID of the device that writes it, and sealed to the queue key of `bob`'s history key of
/// the channel's current epoch, under that epoch; changed as its `alter` says.
#[track_caller]
pub fn sealed_by(
    bob: &Played,
    at: (SpaceId, ChannelId),
    (key, device): (&SigningKey, DeviceId),
    craft: &Craft,
) -> QueuedTask {
    let epoch = bob.mls.epoch(at.1).expect("Bob's epoch");
    let history = (bob.mls.history_key(at.1, epoch))
        .unwrap_or_else(|| panic!("Bob's device holds no key of epoch {epoch}"));
    let pk = history.queue_key().public_key();
    let task = TaskId::generate();
    let request = json!({
        "messageId": task.to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": craft.text }],
    });
    let mut content = Task {
        request: request.to_string(),
        unknown: Unknown::default(),
    }
    .content(
        address(&craft.from),
        craft.to.as_deref().map(address),
        craft.hops,
    );
    content.counter = craft.counter.and_then(NonZeroU64::new);
    let sender = match craft.alter {
        Alter::Sender(other) => other,
        _ => device,
    };
    let body = TaskBody {
        sender,
        channel: at.1,
        queue: craft.queue,
        task,
        sched: craft.sched.clone(),
        mode: craft.mode,
        enqueued: now(),
        content,
        unknown: Unknown::default(),
    };
    let mut plaintext = sign_task(&body, key).expect("a task body");
    if craft.alter == Alter::Signature {
        let mut signature = *plaintext.signature();
        signature[0] ^= 1;
        plaintext = TaskPlaintext::from_parts(plaintext.body_bytes().to_vec(), signature)
            .expect("a task plaintext");
    }
    let mut record = seal_task(&plaintext, epoch, &pk).expect("a sealed task");
    match craft.alter {
        Alter::Ct => record.ct[0] ^= 1,
        Alter::RecordSched => record.sched.idempotency_key = Some(b"altered-sched".to_vec()),
        _ => {}
    }
    record
}

/// `person`'s device uploads `record` to its queue of `at`'s channel; fails unless the server
/// stores it.
#[track_caller]
pub fn uploads(cloud: &Cloud, person: &Person, at: (SpaceId, ChannelId), record: &QueuedTask) {
    let path = tasks_path(at, record.queue);
    let body = Some(("application/cbor", record.to_cbor()));
    let (status, answer) = proved_with(cloud, person, ("POST", &path), body);
    assert!(
        (200..300).contains(&status),
        "{} not stored: {status} {answer}",
        record.task
    );
}

/// A compromised server changes its own copy of the task `task`: `set`, an SQL assignment to
/// columns of its row in the table `tasks`.
pub fn tampers(cloud: &Cloud, task: TaskId, set: &str) {
    cloud.execute(&format!("UPDATE tasks SET {set} WHERE id = '{task}'::uuid"));
}

/// A compromised server gives `record` as the bytes of its task.
pub fn gives_bytes(cloud: &Cloud, record: &QueuedTask) {
    let bytes = hex(&record.to_cbor());
    tampers(
        cloud,
        record.task,
        &format!("queued_task = '\\x{bytes}'::bytea"),
    );
}

/// A task's last claim as the server gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub claim_id: ClaimId,
    pub generation: u64,
    pub device: DeviceId,
}

/// A task as the server gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub state: String,
    pub progress: Option<String>,
    pub reason: Option<String>,
    pub attempts: u64,
    pub claim: Option<Held>,
}

impl Read {
    /// Its last claim; fails unless it had one.
    #[track_caller]
    pub fn held(&self) -> &Held {
        self.claim
            .as_ref()
            .unwrap_or_else(|| panic!("no claim: {self:?}"))
    }

    /// Its state, progress or reason, and attempts.
    pub fn summary(&self) -> (&str, Option<&str>, u64) {
        let detail = self.progress.as_deref().or(self.reason.as_deref());
        (self.state.as_str(), detail, self.attempts)
    }
}

/// The task `task` of `at`'s channel's queue `queue`, as the server gives it to `person`'s device.
#[track_caller]
pub fn read(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    (queue, task): (QueueId, TaskId),
) -> Read {
    let path = format!("{}/{task}", tasks_path(at, queue));
    let (status, body) = proved(cloud, person, ("GET", &path), None);
    assert_eq!(status, 200, "GET {path}: {body}");
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let claim = (body.get("claim").filter(|claim| !claim.is_null())).map(|claim| Held {
        claim_id: (text(&claim["claim_id"]).and_then(|id| id.parse().ok()))
            .unwrap_or_else(|| panic!("no claim ID in {body}")),
        generation: (claim["generation"].as_u64())
            .unwrap_or_else(|| panic!("no generation in {body}")),
        device: (text(&claim["device"]).and_then(|id| id.parse().ok()))
            .unwrap_or_else(|| panic!("no device in {body}")),
    });
    Read {
        state: text(&body["state"]).unwrap_or_else(|| panic!("no state in {body}")),
        progress: text(&body["progress"]),
        reason: text(&body["reason"]),
        attempts: (body["attempts"].as_u64()).unwrap_or_else(|| panic!("no attempts in {body}")),
        claim,
    }
}

/// The first line of each request a machine sent from `since` on that names the task `task`, in
/// lowercase.
pub fn requests_naming(cloud: &Cloud, since: Instant, task: TaskId) -> Vec<String> {
    let named = format!("/tasks/{task}/");
    (cloud.relay.sent_since(since).into_iter())
        .filter_map(|sent| sent.head.lines().next().map(str::to_owned))
        .filter(|line| line.contains(&named))
        .collect()
}

/// Whether one of `requests`, as [`requests_naming`] gives them, ends in `act`, such as `release`.
pub fn sent_act(requests: &[String], act: &str) -> bool {
    let ending = format!("/{act} ");
    requests.iter().any(|line| line.contains(&ending))
}

/// The line that starts a claim from `at`'s channel's queue `queue`, as the relay's rules match it.
pub fn claim_request(at: (SpaceId, ChannelId), queue: QueueId) -> String {
    format!("POST {}/{queue}/claims ", queues_path(at))
}

/// The relay answers every claim from the queue of `record` in `at`'s channel itself, until it
/// stops refusing, with a claim of `record` uploaded by `uploader`, as a compromised server would
/// hand it whatever the claim stated; gives the relay's mark from then on.
pub fn hands_out(
    cloud: &Cloud,
    at: (SpaceId, ChannelId),
    uploader: DeviceId,
    record: &QueuedTask,
) -> Instant {
    let claim = Claim {
        claim_id: ClaimId::generate(),
        generation: 1,
        attempt: 1,
        lease: 300,
        mode: QueueMode::AtLeastOnce,
        uploader,
        task: record.clone(),
        unknown: Unknown::default(),
    };
    let request = claim_request(at, record.queue);
    let answer = ("application/cbor", claim.to_cbor());
    cloud
        .relay
        .answer_with(&request, "200 OK", (answer.0, &answer.1));
    cloud.relay.mark()
}

/// The line that starts a complete of the task `task` of `at`'s channel's queue `queue`.
pub fn complete_request(at: (SpaceId, ChannelId), queue: QueueId, task: TaskId) -> String {
    format!("POST {}/{task}/complete ", tasks_path(at, queue))
}

/// Whether `shown` holds a reference, `#<ref>`, to the task `task`, or its ID.
pub fn names_task(shown: &str, task: TaskId) -> bool {
    shown.contains(&task.to_string()) || refers_to_task(shown, task)
}

/// Whether a line of `shown` says `warning` and names the task `task`.
pub fn warns_of(shown: &str, task: TaskId) -> bool {
    (shown.lines()).any(|line| line.to_lowercase().contains("warning") && names_task(line, task))
}

/// Whether a line of `shown` says `word`, in any case.
pub fn says(shown: &str, word: &str) -> bool {
    shown.to_lowercase().contains(word)
}

/// A result a device read: its message's `msg_id`, the sending device, the content and its
/// `task-result` body.
#[derive(Debug, Clone)]
pub struct Got {
    pub msg_id: MessageId,
    pub sender: DeviceId,
    pub content: Content,
    pub result: TaskResult,
}

/// The `task-result` body of `content`, if it has one.
pub fn result_of(content: &Content) -> Option<TaskResult> {
    let bytes = content.unknown.get(TASK_RESULT)?;
    TaskResult::from_cbor(bytes).ok()
}

/// The results of `task` that Bob's device, the talk's first played one, read in the talk's
/// channel, once it took in its mailbox.
pub fn results_read(talk: &mut Talk, task: TaskId) -> Vec<Got> {
    talk.played[0].take_in(&talk.world.cloud);
    (talk.played[0].read_in(talk.channel).into_iter())
        .filter_map(|(msg_id, decrypted)| {
            let content = Content::from_cbor(&decrypted.plaintext).ok()?;
            let result = result_of(&content)?;
            (result.task == task).then_some(Got {
                msg_id,
                sender: decrypted.sender,
                content,
                result,
            })
        })
        .collect()
}

/// What a result must be.
pub struct Expected<'a> {
    /// The claiming device.
    pub sender: DeviceId,
    /// `from`, the claiming agent; `to`, the task's `from`; and `hops`.
    pub from: &'a str,
    pub to: &'a str,
    pub hops: u64,
    /// The task and its claim as the server gives them.
    pub task: TaskId,
    pub held: &'a Held,
    pub outcome: Outcome,
    /// The one text part of `response`.
    pub text: &'a str,
}

/// Fails unless `got` is as `expected` says and "Results" says: sent by the claiming device, from
/// the claiming agent to the task's `from`, with the `task-result` body alone, naming the task, the
/// claim and its generation, the outcome, and a `response` of the role `ROLE_AGENT` whose
/// `messageId` is the message's `msg_id`.
#[track_caller]
pub fn assert_result(got: &Got, expected: &Expected) {
    let (content, result) = (&got.content, &got.result);
    assert_eq!(got.sender, expected.sender, "the sending device: {got:?}");
    assert_eq!(content.from.to_string(), expected.from, "{content:?}");
    assert_eq!(
        content.to.as_ref().map(ToString::to_string).as_deref(),
        Some(expected.to),
        "{content:?}"
    );
    assert_eq!(content.hops, expected.hops, "the hops of {content:?}");
    assert_eq!(content.body, None, "another body in {content:?}");
    assert_eq!(
        (result.task, result.claim_id, result.generation),
        (
            expected.task,
            expected.held.claim_id,
            expected.held.generation
        ),
        "the task and claim of {result:?}"
    );
    assert_eq!(result.outcome, expected.outcome, "{result:?}");
    let response: Value = serde_json::from_str(&result.response)
        .unwrap_or_else(|err| panic!("a response not in JSON: {err}: {result:?}"));
    assert_eq!(response["role"].as_str(), Some("ROLE_AGENT"), "{response}");
    assert_eq!(
        response["messageId"].as_str(),
        Some(got.msg_id.to_string().as_str()),
        "{response}"
    );
    let texts: Vec<&str> = (response["parts"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect();
    assert_eq!(texts, [expected.text], "the text parts of {response}");
}

/// Runs `skald recv` as `agent` on `machine` in `dir`; fails unless it succeeds.
#[track_caller]
pub fn agent_reads(machine: &Machine, dir: &Path, agent: &str) -> Finished {
    run_as_in(machine, dir, agent)
}
