//! The items of `docs/protocol/queues.md` apart from the crates under test: maps of their spec
//! fields, encoded with the tests' CBOR writer, and signed objects signed by the tests' devices
//! over `[label, body]`.

use skald_proto::{ChannelId, ClaimId, QueueId, TaskId};

use crate::cbor::{Value, bytes, text};
use crate::devices::{NOW, device_id, signing_key};
use crate::fields::{signed_cbor, unknown};

/// The labels of a task plaintext's and a queue key object's signatures.
pub const TASK_LABEL: &str = "skald task";
pub const QUEUE_KEY_LABEL: &str = "skald queue key";

/// `queue-mode` and `outcome` as the spec encodes them.
pub const AT_LEAST_ONCE: u64 = 1;
pub const AT_MOST_ONCE: u64 = 2;
pub const DONE: u64 = 1;
pub const FAILED: u64 = 2;

/// The tests' devices, by seed: the sending device, the addressee's device and a member device
/// that signs a queue key object.
pub const SENDER: u8 = 81;
pub const ADDRESSEE: u8 = 82;
pub const SIGNER: u8 = 83;

/// The enqueuing agent and the addressee agent.
pub const FROM: &str = "acme/alice/claude-api@skald.example";
pub const TO: &str = "acme/bob/codex@skald.example";

pub const IDEMPOTENCY_KEY: &[u8] = b"build-1842";
pub const TEMPLATE: &str = "review";
/// When the sender enqueued the task, and its deadline 7 days later.
pub const ENQUEUED: u64 = NOW;
pub const DEADLINE: u64 = NOW + 7 * 86_400;
/// The epoch whose queue key seals the task.
pub const EPOCH: u64 = 3;
pub const ENC: [u8; 32] = [0xe5; 32];
pub const CT: [u8; 40] = [0xc7; 40];
/// A queue key object's `key`.
pub const KEY: [u8; 32] = [0x9b; 32];
pub const GENERATION: u64 = 4;
pub const ATTEMPT: u64 = 2;
pub const LEASE: u64 = 300;

/// The IDs of one task: its channel, queue, task and claim.
#[derive(Debug, Clone, Copy)]
pub struct Ids {
    pub channel: ChannelId,
    pub queue: QueueId,
    pub task: TaskId,
    pub claim: ClaimId,
}

impl Ids {
    pub fn new() -> Self {
        Self {
            channel: ChannelId::generate(),
            queue: QueueId::generate(),
            task: TaskId::generate(),
            claim: ClaimId::generate(),
        }
    }
}

/// `fields` with the fields of later versions of [`unknown`].
pub fn with_unknown(mut fields: Vec<(i64, Value)>) -> Vec<(i64, Value)> {
    fields.extend(unknown());
    fields
}

/// A `sched` with every scheduling field: an idempotency key, a template, a deadline and the
/// addressee's device.
pub fn sched_fields() -> Vec<(i64, Value)> {
    vec![
        (1, bytes(IDEMPOTENCY_KEY)),
        (4, text(TEMPLATE)),
        (5, Value::Uint(DEADLINE)),
        (6, bytes(device_id(ADDRESSEE).as_bytes())),
    ]
}

/// The fields of a `queued-task` of `ids` with this `sched`.
pub fn queued_fields(ids: &Ids, sched: Vec<(i64, Value)>) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(ids.task.as_bytes())),
        (2, bytes(ids.queue.as_bytes())),
        (3, Value::Map(sched)),
        (4, Value::Uint(EPOCH)),
        (5, bytes(&ENC)),
        (6, bytes(&CT)),
    ]
}

/// A `queued-task` of `ids` with every scheduling field and fields of later versions in both maps.
pub fn rich_queued(ids: &Ids) -> Vec<(i64, Value)> {
    with_unknown(queued_fields(ids, with_unknown(sched_fields())))
}

/// The A2A Message of a task's `request`: its `messageId` the task in text form, its role
/// `ROLE_USER`.
pub fn request(task: TaskId) -> String {
    format!(
        r#"{{"messageId":"{task}","contextId":"019a6f20-0000-7000-8000-00000000c0de","role":"ROLE_USER","parts":[{{"text":"Review PR 12"}}]}}"#
    )
}

/// The A2A Message of a result's `response`.
pub fn response() -> String {
    r#"{"messageId":"019a6f20-1a00-7c00-8a00-0000000000aa","contextId":"019a6f20-0000-7000-8000-00000000c0de","role":"ROLE_AGENT","parts":[{"text":"Approved"}]}"#.to_owned()
}

/// The fields of a `task` body with the request of `task`.
pub fn task_fields(task: TaskId) -> Vec<(i64, Value)> {
    vec![(1, text(&request(task)))]
}

/// The encoded content of a task of `ids` to the addressee agent: no counter, hops 0, a `task`
/// body.
pub fn content(ids: &Ids) -> Vec<u8> {
    Value::Map(vec![
        (1, text(FROM)),
        (2, text(TO)),
        (4, Value::Uint(0)),
        (11, Value::Map(task_fields(ids.task))),
    ])
    .encode()
}

/// The fields of a `task-body` of `ids` sent by [`SENDER`], with this `sched` and `mode`.
pub fn task_body_fields(ids: &Ids, sched: Vec<(i64, Value)>, mode: u64) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(device_id(SENDER).as_bytes())),
        (2, bytes(ids.channel.as_bytes())),
        (3, bytes(ids.queue.as_bytes())),
        (4, bytes(ids.task.as_bytes())),
        (5, Value::Map(sched)),
        (6, Value::Uint(mode)),
        (7, Value::Uint(ENQUEUED)),
        (8, bytes(&content(ids))),
    ]
}

/// The fields of a `task-result` of `ids` with this `outcome`.
pub fn result_fields(ids: &Ids, outcome: u64) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(ids.task.as_bytes())),
        (2, bytes(ids.claim.as_bytes())),
        (3, Value::Uint(GENERATION)),
        (4, Value::Uint(outcome)),
        (5, text(&response())),
    ]
}

/// The fields of a `claim` of `ids` in a queue of `mode`, uploaded by [`SENDER`], with this
/// `queued-task`.
pub fn claim_fields(ids: &Ids, mode: u64, queued: Vec<(i64, Value)>) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(ids.claim.as_bytes())),
        (2, Value::Uint(GENERATION)),
        (3, Value::Uint(ATTEMPT)),
        (4, Value::Uint(LEASE)),
        (5, Value::Uint(mode)),
        (6, bytes(device_id(SENDER).as_bytes())),
        (7, Value::Map(queued)),
    ]
}

/// The fields of a `queue-key-body` of `ids`'s channel in [`EPOCH`], signed by [`SIGNER`].
pub fn queue_key_body_fields(ids: &Ids) -> Vec<(i64, Value)> {
    vec![
        (1, bytes(ids.channel.as_bytes())),
        (2, Value::Uint(EPOCH)),
        (3, bytes(&KEY)),
        (4, bytes(device_id(SIGNER).as_bytes())),
    ]
}

/// The encoded map of `fields`.
pub fn encoded(fields: &[(i64, Value)]) -> Vec<u8> {
    Value::Map(fields.to_vec()).encode()
}

/// The encoded signed object of the encoded `body`, signed by device `seed` over `[label, body]`.
pub fn signed(body: &[u8], seed: u8, label: &str) -> Vec<u8> {
    signed_cbor(body, &signing_key(seed).sign(label, &[body]))
}
