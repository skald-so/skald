//! What the tests share: the queue commands as the person or an agent runs them, a device's proved
//! requests for a channel's queues and tasks, the tasks the machines uploaded as the relay saw
//! them, and a task opened as Bob's device opens it.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use skald_crypto::check_task;
use skald_proto::{
    Cbor as _, ChannelId, Content, DeviceId, MessageId, PROOF_HEADER, PROTOCOL_VERSION, QueueId,
    QueuedTask, REQUEST_LABEL, RequestProof, SpaceId, Task, TaskBody, TaskId, VERSION_HEADER,
};

use crate::cloud::{Cloud, agent, unb64};
use crate::fixtures::{cert_of, hex};
use crate::lines::refers_to;
use crate::machine::{Finished, Machine};
use crate::talk::Talk;
use crate::world::{AGENT, Person, post_as, words};

/// The queue commands.
pub const QUEUES: &str = "queues";
pub const CREATE_QUEUE: &str = "create-queue";
pub const CHANGE_QUEUE: &str = "change-queue";
pub const DELETE_QUEUE: &str = "delete-queue";
pub const ENQUEUE: &str = "enqueue";

/// The options of the queue commands, beside `--channel` and `--to` of `skald send`.
pub const QUEUE_FLAG: &str = "--queue";
pub const MODE_FLAG: &str = "--mode";
pub const LEASE_FLAG: &str = "--lease";
pub const MAX_ATTEMPTS_FLAG: &str = "--max-attempts";
pub const IDEMPOTENCY_KEY_FLAG: &str = "--idempotency-key";
pub const DEADLINE_FLAG: &str = "--deadline";

/// The queue every channel has, and its default settings.
pub const DEFAULT: &str = "default";
pub const AT_LEAST_ONCE: &str = "at-least-once";
pub const AT_MOST_ONCE: &str = "at-most-once";
pub const LEASE: u64 = 300;
pub const MAX_ATTEMPTS: u64 = 5;

/// The problems the CLI names.
pub const NOT_ALLOWED: &str = "not-allowed";
pub const DUPLICATE: &str = "duplicate";

/// What a refusal under an unclean epoch names.
pub const EPOCH: &str = "epoch";

/// The span from enqueue to the `deadline` a device sets, in seconds.
pub const WEEK: u64 = 7 * 24 * 60 * 60;

/// How far a time the device states may be from the tests' clock, in seconds.
pub const SLACK: u64 = 3;

/// Runs `[--agent <agent>] <command> <args>` on `machine` in `dir`: as `agent`, or as the person
/// when it is `None`.
pub fn run_in(
    machine: &Machine,
    (dir, agent): (&Path, Option<&str>),
    command: &str,
    args: &[&str],
) -> Finished {
    machine
        .start_in(dir, &as_agent(agent, command, args))
        .finish()
}

/// The arguments `[--agent <agent>] <command> <args>`.
pub fn as_agent<'a>(agent: Option<&'a str>, command: &'a str, args: &[&'a str]) -> Vec<&'a str> {
    let mut all = Vec::new();
    if let Some(agent) = agent {
        all.extend([AGENT, agent]);
    }
    all.push(command);
    all.extend_from_slice(args);
    all
}

/// Runs `[--agent <agent>] enqueue <args>` on `machine` in `dir`.
pub fn enqueue(machine: &Machine, at: (&Path, Option<&str>), args: &[&str]) -> Finished {
    run_in(machine, at, ENQUEUE, args)
}

/// As [`enqueue`]; fails unless it succeeds.
#[track_caller]
pub fn enqueued(machine: &Machine, at: (&Path, Option<&str>), args: &[&str]) -> Finished {
    let ran = enqueue(machine, at, args);
    assert!(ran.success, "{args:?}: {ran:?}");
    ran
}

/// Whether a line of `finished` holds all of `parts` as words.
pub fn line_with(finished: &Finished, parts: &[&str]) -> bool {
    let shown = finished.shown().to_lowercase();
    (shown.lines()).any(|line| {
        let found = words(line);
        parts.iter().all(|part| found.contains(part))
    })
}

/// Whether a line of `finished` lists the queue `name` with `mode`, `lease` and `max_attempts`.
pub fn lists_queue(
    finished: &Finished,
    (name, mode, lease, max_attempts): (&str, &str, u64, u64),
) -> bool {
    let (lease, max_attempts) = (lease.to_string(), max_attempts.to_string());
    line_with(finished, &[name, mode, &lease, &max_attempts])
}

/// Whether a line of `finished` names the queue `name` as a word.
pub fn names_queue(finished: &Finished, name: &str) -> bool {
    line_with(finished, &[name])
}

/// Whether `finished` names `problem` as a word.
pub fn names_problem(finished: &Finished, problem: &str) -> bool {
    line_with(finished, &[problem])
}

/// The references in `shown`: each run of letters and digits after a `#`.
pub fn references(shown: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in shown.match_indices('#') {
        let rest = &shown[at + 1..];
        let end = rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(rest.len());
        if end > 0 {
            found.push(rest[..end].to_owned());
        }
    }
    found
}

/// Whether `shown` holds a reference, `#<ref>`, to the task `task`.
pub fn refers_to_task(shown: &str, task: TaskId) -> bool {
    let as_message = MessageId::from_bytes(*task.as_bytes());
    references(shown)
        .iter()
        .any(|short| refers_to(short, as_message))
}

/// Fails unless `finished`, a run of `skald enqueue`, says the reference of `task`.
#[track_caller]
pub fn assert_says_reference(finished: &Finished, task: TaskId) {
    assert!(
        refers_to_task(&finished.shown(), task),
        "no #<ref> of {task}: {finished:?}"
    );
}

/// The answer to `<method> <path>`, with the JSON `body` if any, on `person`'s session, proving
/// `person`'s device on a new challenge: the status and the body.
pub fn proved(
    cloud: &Cloud,
    person: &Person,
    (method, path): (&str, &str),
    body: Option<&Value>,
) -> (u16, Value) {
    let body = body.map(|json| ("application/json", json.to_string().into_bytes()));
    proved_with(cloud, person, (method, path), body)
}

/// As [`proved`], with the body `(media type, bytes)` if any, such as a task's CBOR bytes.
pub fn proved_with(
    cloud: &Cloud,
    person: &Person,
    (method, path): (&str, &str),
    body: Option<(&str, Vec<u8>)>,
) -> (u16, Value) {
    let header = proof_of(cloud, person);
    let mut request = ureq::http::Request::builder()
        .method(method)
        .uri(format!("{}{path}", cloud.url))
        .header(VERSION_HEADER, PROTOCOL_VERSION.to_string())
        .header("authorization", format!("Bearer {}", person.token))
        .header(PROOF_HEADER, header);
    let sent = match body {
        Some((media, bytes)) => {
            request = request.header("content-type", media);
            agent().run(request.body(bytes).expect("a request"))
        }
        None => agent().run(request.body(()).expect("a request")),
    };
    let mut answer = sent.unwrap_or_else(|err| panic!("{method} {path}: {err}"));
    let status = answer.status().as_u16();
    let text = answer.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// The `PROOF_HEADER` of a request of `person`'s device: a fresh challenge the server issues,
/// signed with the device's key over the server's public origin.
#[track_caller]
fn proof_of(cloud: &Cloud, person: &Person) -> String {
    let (code, answer) = post_as(cloud, &person.token, "/session/challenge", None);
    let challenge = match (code, answer["challenge"].as_str()) {
        (200, Some(issued)) => unb64(issued),
        _ => panic!("{} got no challenge: {code} {answer}", person.host),
    };
    let signed = [challenge.as_slice(), cloud.public.as_bytes()];
    let signature = person.key.sign(REQUEST_LABEL, &signed);
    let device = person.device;
    let proof = RequestProof {
        signature,
        challenge,
        device,
    };
    URL_SAFE_NO_PAD.encode(proof.to_cbor())
}

/// The path of `at`'s channel's queues.
pub fn queues_path((space, channel): (SpaceId, ChannelId)) -> String {
    format!("/spaces/{space}/channels/{channel}/queues")
}

/// The path of the tasks of `at`'s channel's queue `queue`.
pub fn tasks_path(at: (SpaceId, ChannelId), queue: QueueId) -> String {
    format!("{}/{queue}/tasks", queues_path(at))
}

/// A queue as the server gives it: its ID, name, mode, lease and `max_attempts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub id: QueueId,
    pub name: String,
    pub mode: String,
    pub lease: u64,
    pub max_attempts: u64,
}

/// The queue `value`, an item of the server's answer; fails unless it has every field.
#[track_caller]
fn queue_of(value: &Value) -> Queue {
    let field = |name: &str| (value.get(name)).unwrap_or_else(|| panic!("no {name} in {value}"));
    let text = |name: &str| field(name).as_str().unwrap_or_default().to_owned();
    let count = |name: &str| field(name).as_u64().unwrap_or_default();
    let id = text("id")
        .parse()
        .unwrap_or_else(|_| panic!("no queue ID in {value}"));
    let (name, mode) = (text("name"), text("mode"));
    let (lease, max_attempts) = (count("lease"), count("max_attempts"));
    Queue {
        id,
        name,
        mode,
        lease,
        max_attempts,
    }
}

impl Queue {
    /// Its settings: mode, lease and `max_attempts`.
    pub fn settings(&self) -> (&str, u64, u64) {
        (self.mode.as_str(), self.lease, self.max_attempts)
    }
}

/// The queues of `at`'s channel the server gives `person`'s device, sorted by name.
#[track_caller]
pub fn queues(cloud: &Cloud, person: &Person, at: (SpaceId, ChannelId)) -> Vec<Queue> {
    let path = queues_path(at);
    let (status, body) = proved(cloud, person, ("GET", &path), None);
    let Some(listed) = (status == 200).then(|| body["queues"].as_array()).flatten() else {
        panic!("GET {path} as {}: {status} {body}", person.host);
    };
    let mut queues: Vec<Queue> = listed.iter().map(queue_of).collect();
    queues.sort_by_key(|queue| queue.name.clone());
    queues
}

/// The queue `name` of `at`'s channel, as the server gives it to `person`'s device, if any.
#[track_caller]
pub fn queue_named(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    name: &str,
) -> Option<Queue> {
    queues(cloud, person, at)
        .into_iter()
        .find(|queue| queue.name == name)
}

/// The ID of `at`'s channel's queue `default`.
#[track_caller]
pub fn default_queue(cloud: &Cloud, person: &Person, at: (SpaceId, ChannelId)) -> QueueId {
    let queue = queue_named(cloud, person, at, DEFAULT);
    queue
        .unwrap_or_else(|| panic!("no queue {DEFAULT} in {}", at.1))
        .id
}

/// `person`'s device creates a queue of `settings` in `at`'s channel through the API; fails unless
/// the server creates it. Gives it.
#[track_caller]
pub fn creates_queue(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    settings: &Value,
) -> Queue {
    let path = queues_path(at);
    let (status, body) = proved(cloud, person, ("POST", &path), Some(settings));
    match status {
        201 => queue_of(&body),
        _ => panic!("POST {path} {settings} as {}: {status} {body}", person.host),
    }
}

/// A task as the server gives it: the bytes its device uploaded, decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub bytes: Vec<u8>,
    pub task: QueuedTask,
    pub state: String,
}

/// The tasks of `at`'s channel's queue `queue` the server gives `person`'s device.
#[track_caller]
pub fn tasks(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    queue: QueueId,
) -> Vec<Stored> {
    let path = tasks_path(at, queue);
    let (status, body) = proved(cloud, person, ("GET", &path), None);
    assert_eq!(status, 200, "GET {path} as {}: {body}", person.host);
    let items = (body["tasks"].as_array()).unwrap_or_else(|| panic!("no tasks in {body}"));
    items
        .iter()
        .map(|item| {
            let encoded = (item["queued_task"].as_str())
                .unwrap_or_else(|| panic!("no queued_task in {item}"));
            let bytes = unb64(encoded);
            let task = QueuedTask::from_cbor(&bytes)
                .unwrap_or_else(|err| panic!("a task of {path} that does not decode: {err}"));
            Stored {
                bytes,
                task,
                state: item["state"].as_str().unwrap_or_default().to_owned(),
            }
        })
        .collect()
}

/// The one task of `at`'s channel's queue `queue` the server gives `person`'s device; fails unless
/// there is one.
#[track_caller]
pub fn the_one_task(
    cloud: &Cloud,
    person: &Person,
    at: (SpaceId, ChannelId),
    queue: QueueId,
) -> Stored {
    let mut held = tasks(cloud, person, at, queue);
    assert_eq!(held.len(), 1, "not one task in {queue}: {held:?}");
    held.remove(0)
}

/// The task among `held` that `finished`, a run of `skald enqueue`, refers to; fails unless one
/// is.
#[track_caller]
pub fn referred(held: &[Stored], finished: &Finished) -> Stored {
    let shown = finished.shown();
    let found: Vec<&Stored> = (held.iter())
        .filter(|stored| refers_to_task(&shown, stored.task.task))
        .collect();
    match found.as_slice() {
        [one] => (*one).clone(),
        _ => panic!("not one task referred to among {held:?}: {finished:?}"),
    }
}

/// The first line of each request a machine sent from `since` on, in lowercase.
fn first_lines(cloud: &Cloud, since: std::time::Instant) -> Vec<String> {
    (cloud.relay.sent_since(since).into_iter())
        .filter_map(|sent| sent.head.lines().next().map(str::to_owned))
        .collect()
}

/// The first line of each request a machine sent from `since` on that uploads a task.
pub fn task_uploads(cloud: &Cloud, since: std::time::Instant) -> Vec<String> {
    let uploads = |line: &String| {
        line.starts_with("post /spaces/") && line.contains("/queues/") && line.contains("/tasks ")
    };
    first_lines(cloud, since)
        .into_iter()
        .filter(uploads)
        .collect()
}

/// Fails if a machine sent a request uploading a task from `since` on.
#[track_caller]
pub fn assert_none_uploaded(cloud: &Cloud, since: std::time::Instant, what: &str) {
    let uploads = task_uploads(cloud, since);
    assert!(uploads.is_empty(), "{what}: a task uploaded: {uploads:?}");
}

/// The first line of each request a machine sent from `since` on that creates, changes or deletes
/// a queue.
pub fn queue_writes(cloud: &Cloud, since: std::time::Instant) -> Vec<String> {
    let writes = |line: &String| {
        let (method, path) = line.split_once(' ').unwrap_or_default();
        let path = path.split(' ').next().unwrap_or_default();
        ["post", "patch", "delete", "put"].contains(&method)
            && path.contains("/queues")
            && !path.contains("/tasks")
    };
    first_lines(cloud, since)
        .into_iter()
        .filter(writes)
        .collect()
}

/// The line that starts the request uploading a task to `at`'s channel's queue `queue`, as the
/// relay's rules match it.
pub fn upload_request(at: (SpaceId, ChannelId), queue: QueueId) -> String {
    format!("POST {} ", tasks_path(at, queue))
}

/// How many times `needle` is in `haystack`.
pub fn occurrences(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Fails if `text` is in a byte the machines sent or, in either form, in a row of the server's
/// database: a task's text reaches the server only sealed.
#[track_caller]
pub fn assert_never_in_the_clear(cloud: &Cloud, text: &str) {
    let sent = cloud.relay.sent_bytes();
    assert_eq!(
        occurrences(&sent, text.as_bytes()),
        0,
        "{text:?} sent in the clear"
    );
    let in_hex = hex(text.as_bytes());
    let rows = cloud.rows();
    assert!(
        rows.iter()
            .all(|table| !table.contains(text) && !table.to_lowercase().contains(&in_hex)),
        "{text:?} in the server's database"
    );
}

/// A task opened as Bob's device, the talk's first played one, opens it: its body, and its
/// content's `task` body.
#[derive(Debug, Clone)]
pub struct Opened {
    pub body: TaskBody,
    pub task: Task,
}

impl Opened {
    pub fn content(&self) -> &Content {
        &self.body.content
    }

    /// The request, in JSON.
    #[track_caller]
    pub fn request(&self) -> Value {
        serde_json::from_str(&self.task.request)
            .unwrap_or_else(|err| panic!("a request not in JSON: {err}: {:?}", self.task))
    }
}

/// What a test of Ada's tasks in the talk's channel needs.
pub struct Sending {
    /// The talk's channel.
    pub at: (SpaceId, ChannelId),
    /// Its ID, as a command line names it.
    pub channel: String,
    /// Its queue `default`.
    pub queue: QueueId,
    /// Ada's device, and its signing key as the server gives its certificate.
    pub device: DeviceId,
    pub signing: [u8; 32],
}

/// What a test of Ada's tasks in `talk`'s channel needs.
#[track_caller]
pub fn sending(talk: &Talk) -> Sending {
    let (at, cloud, ada) = (talk.at(), talk.cloud(), &talk.ada);
    Sending {
        at,
        channel: at.1.to_string(),
        queue: default_queue(cloud, ada, at),
        device: ada.device,
        signing: signing_of(cloud, ada),
    }
}

/// The signing key of the certificate of `person`'s device, as the server gives it.
#[track_caller]
pub fn signing_of(cloud: &Cloud, person: &Person) -> [u8; 32] {
    cert_of(cloud, person).body().signing
}

/// `stored`, a task of `at`'s channel's queue `queue`, opened with the queue key of Bob's history
/// key of its epoch, once Bob's device took in its mailbox, and its signature checked with
/// `signing`, the sending device's signing key; fails unless it opens and verifies, and its
/// content holds a `task` body.
#[track_caller]
pub fn opened(
    talk: &mut Talk,
    at: (SpaceId, ChannelId),
    (queue, stored): (QueueId, &Stored),
    signing: &[u8; 32],
) -> Opened {
    talk.played[0].take_in(&talk.world.cloud);
    let task = &stored.task;
    let bob = &talk.played[0];
    let Some(key) = bob.mls.history_key(at.1, task.epoch) else {
        panic!("Bob's device holds no key of epoch {}", task.epoch);
    };
    let plaintext = (key.queue_key().open_task(task, at.1, queue))
        .unwrap_or_else(|err| panic!("{} does not open: {err}", task.task));
    check_task(&plaintext, signing)
        .unwrap_or_else(|_| panic!("{} is not signed by the sending device", task.task));
    let body = plaintext.body().clone();
    let bytes = (body.content.unknown.get(11))
        .unwrap_or_else(|| panic!("no task body in {:?}", body.content))
        .to_vec();
    let inner = Task::from_cbor(&bytes)
        .unwrap_or_else(|err| panic!("a task body that does not decode: {err}"));
    Opened { body, task: inner }
}

/// Fails unless `opened`, the task `stored` of `at`'s channel's queue `queue`, is as "Tasks" says:
/// sent by the device `sender` in that channel and queue, its ID the record's, the record's `sched`,
/// enqueued within `when` as the tests' clock gives it, its content from `from` with a `task` body
/// and no `counter`, and a request of the role `ROLE_USER` whose `messageId` is the task's ID and
/// whose one text part is `text`.
#[track_caller]
pub fn assert_task(
    opened: &Opened,
    (at, queue, stored): ((SpaceId, ChannelId), QueueId, &Stored),
    (sender, from, text): (DeviceId, &str, &str),
    when: (u64, u64),
) {
    let (body, task) = (&opened.body, &stored.task);
    assert_eq!(body.sender, sender, "the sender of {}", task.task);
    assert_eq!(body.channel, at.1, "the channel of {}", task.task);
    assert_eq!(body.queue, queue, "the queue of {}", task.task);
    assert_eq!(task.queue, queue, "the record's queue of {}", task.task);
    assert_eq!(body.task, task.task, "the ID of {}", task.task);
    assert_eq!(body.sched, task.sched, "the sched of {}", task.task);
    assert!(
        when.0.saturating_sub(SLACK) <= body.enqueued && body.enqueued <= when.1 + SLACK,
        "{} enqueued at {}, not within {when:?}",
        task.task,
        body.enqueued
    );
    let content = opened.content();
    assert_eq!(content.from.to_string(), from, "{content:?}");
    assert_eq!(content.counter, None, "a counter in {content:?}");
    assert_eq!(content.body, None, "another body in {content:?}");
    let request = opened.request();
    assert_eq!(request["role"].as_str(), Some("ROLE_USER"), "{request}");
    assert_eq!(
        request["messageId"].as_str(),
        Some(task.task.to_string().as_str()),
        "{request}"
    );
    let texts: Vec<&str> = (request["parts"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect();
    assert_eq!(texts, [text], "the text parts of {request}");
}

/// Fails unless the record of `stored` holds nothing but its fields: no field of a later version,
/// in it or in its `sched`.
#[track_caller]
pub fn assert_plain_record(stored: &Stored) {
    assert!(stored.task.unknown.is_empty(), "more fields in {stored:?}");
    assert!(
        stored.task.sched.unknown.is_empty(),
        "more fields in {:?}",
        stored.task.sched
    );
    assert_eq!(stored.task.to_cbor(), stored.bytes, "{stored:?}");
}

/// `seconds` since the Unix epoch as an RFC 3339 time, such as `2027-01-31T00:00:00Z`.
pub fn rfc3339(seconds: u64) -> String {
    let seconds = i64::try_from(seconds).expect("a time within range");
    jiff::Timestamp::from_second(seconds)
        .expect("a time within range")
        .to_string()
}

/// Fails unless `deadline` is a week after `enqueued`, and so within a week after `when`, as the
/// tests' clock gives it.
#[track_caller]
pub fn assert_week_after(deadline: Option<u64>, enqueued: u64, when: (u64, u64)) {
    let Some(deadline) = deadline else {
        panic!("no deadline, enqueued at {enqueued}");
    };
    assert!(
        deadline.abs_diff(enqueued + WEEK) <= SLACK,
        "the deadline {deadline} is not a week after {enqueued}"
    );
    assert!(
        when.0 + WEEK - SLACK <= deadline && deadline <= when.1 + WEEK + SLACK,
        "the deadline {deadline} is not a week after {when:?}"
    );
}

/// Fails unless `finished` failed and names each of `parts` as a word, in any case.
#[track_caller]
pub fn assert_refused_naming(finished: &Finished, parts: &[&str], what: &str) {
    assert!(!finished.success, "{what}: not refused: {finished:?}");
    let shown = finished.shown().to_lowercase();
    for part in parts {
        let part = part.to_lowercase();
        assert!(
            shown
                .lines()
                .any(|line| words(line).contains(&part.as_str())),
            "{what}: names no {part}: {finished:?}"
        );
    }
}
