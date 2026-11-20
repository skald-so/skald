//! Queues and tasks: `docs/protocol/queues.md`, "Tasks", "Queue key", "Claims and leases" and
//! "Results".

use crate::address::Name;
use crate::cbor::{self, Codec, DecodeError, FieldProblem, Item, MapOut, Unknown, field_error};
use crate::content::Content;
use crate::history::at_least;
use crate::id::{ChannelId, ClaimId, DeviceId, QueueId, TaskId};
use crate::signed::{Signed, SignedBody};

/// The label of a task plaintext's signature.
pub const TASK_LABEL: &str = "skald task";

/// The label of a queue key object's signature.
pub const QUEUE_KEY_LABEL: &str = "skald queue key";

/// The longest `idempotency_key` of a [`Sched`], in bytes.
pub const IDEMPOTENCY_KEY_MAX: usize = 32;

/// What the sender uploads; the server keeps and returns it unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedTask {
    /// Issued by the sending device.
    pub task: TaskId,
    pub queue: QueueId,
    pub sched: Sched,
    /// Whose queue key seals `ct`; at least 1.
    pub epoch: u64,
    /// The HPKE encapsulated key.
    pub enc: [u8; 32],
    /// The [`TaskPlaintext`], sealed.
    pub ct: Vec<u8>,
    pub unknown: Unknown,
}

impl QueuedTask {
    /// The `aad` of the seal of task `task` in `channel`'s queue `queue` under epoch `epoch`: the
    /// encoded `[channel, queue, task, epoch]`.
    pub fn associated_data(
        channel: ChannelId,
        queue: QueueId,
        task: TaskId,
        epoch: u64,
    ) -> Vec<u8> {
        cbor::array([
            cbor::id(&channel),
            cbor::id(&queue),
            cbor::id(&task),
            cbor::uint(epoch),
        ])
    }
}

/// A task's scheduling fields, in plaintext.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sched {
    /// 1 to 32 bytes.
    pub idempotency_key: Option<Vec<u8>>,
    pub template: Option<Name>,
    /// Seconds since the Unix epoch.
    pub deadline: Option<u64>,
    /// The addressee's device.
    pub device: Option<DeviceId>,
    pub unknown: Unknown,
}

/// What a [`QueuedTask`] seals: its body signed by `sender` over `["skald task", body]`.
pub type TaskPlaintext = Signed<TaskBody>;

/// The body of a [`TaskPlaintext`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskBody {
    /// The sending device.
    pub sender: DeviceId,
    pub channel: ChannelId,
    pub queue: QueueId,
    pub task: TaskId,
    /// The queued task's `sched`.
    pub sched: Sched,
    /// The queue's mode.
    pub mode: QueueMode,
    /// When it was enqueued, as the sender states it, in seconds since the Unix epoch.
    pub enqueued: u64,
    /// With a [`Task`] body.
    pub content: Content,
    pub unknown: Unknown,
}

impl SignedBody for TaskBody {
    const LABEL: &'static str = TASK_LABEL;
    const FIELDS: [&'static str; 3] = [
        "task-plaintext",
        "task-plaintext.body",
        "task-plaintext.signature",
    ];
}

/// What a queue guarantees about running a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueMode {
    AtLeastOnce,
    AtMostOnce,
}

/// The `task` body of content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// One A2A Message in JSON.
    pub request: String,
    pub unknown: Unknown,
}

/// The `task-result` body of content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskResult {
    pub task: TaskId,
    pub claim_id: ClaimId,
    /// At least 1.
    pub generation: u64,
    pub outcome: Outcome,
    /// One A2A Message in JSON.
    pub response: String,
    pub unknown: Unknown,
}

/// How a claim of a task ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    Done,
    Failed,
}

/// A task handed out to a claiming device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    /// Issued by the server for this claim.
    pub claim_id: ClaimId,
    /// At least 1.
    pub generation: u64,
    /// Claims since enqueue or the last restart, released ones not counted; at least 1.
    pub attempt: u64,
    /// Seconds.
    pub lease: u64,
    /// The queue's mode.
    pub mode: QueueMode,
    /// The device that uploaded the task.
    pub uploader: DeviceId,
    pub task: QueuedTask,
    pub unknown: Unknown,
}

/// An epoch's queue key, signed by `signer` over `["skald queue key", body]`.
pub type QueueKey = Signed<QueueKeyBody>;

/// The body of a [`QueueKey`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueKeyBody {
    pub channel: ChannelId,
    /// At least 1.
    pub epoch: u64,
    /// The epoch's `pk`.
    pub key: [u8; 32],
    /// A member device of the epoch.
    pub signer: DeviceId,
    pub unknown: Unknown,
}

impl SignedBody for QueueKeyBody {
    const LABEL: &'static str = QUEUE_KEY_LABEL;
    const FIELDS: [&'static str; 3] = ["queue-key", "queue-key.body", "queue-key.signature"];
}

impl Codec for QueuedTask {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_id(1, &self.task)
            .put_id(2, &self.queue)
            .put(3, self.sched.encode())
            .put(4, cbor::uint(self.epoch))
            .put_bytes(5, &self.enc)
            .put_bytes(6, &self.ct)
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("queued-task")?;
        Ok(Self {
            task: fields
                .required(1, "queued-task.task")?
                .id("queued-task.task")?,
            queue: fields
                .required(2, "queued-task.queue")?
                .id("queued-task.queue")?,
            sched: Sched::decode(fields.required(3, "queued-task.sched")?)?,
            epoch: at_least(
                fields.required(4, "queued-task.epoch")?,
                "queued-task.epoch",
                1,
            )?,
            enc: fields
                .required(5, "queued-task.enc")?
                .fixed("queued-task.enc")?,
            ct: fields
                .required(6, "queued-task.ct")?
                .bytes("queued-task.ct")?
                .to_vec(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Sched {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_some(1, self.idempotency_key.as_deref().map(cbor::bytes))
            .put_some(
                4,
                self.template.as_ref().map(|name| cbor::text(name.as_str())),
            )
            .put_some(5, self.deadline.map(cbor::uint))
            .put_some(6, self.device.as_ref().map(cbor::id))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("sched")?;
        Ok(Self {
            idempotency_key: fields.take(1).map(idempotency_key).transpose()?,
            template: fields
                .take(4)
                .map(|item| item.name("sched.template"))
                .transpose()?,
            deadline: fields
                .take(5)
                .map(|item| item.uint("sched.deadline"))
                .transpose()?,
            device: fields
                .take(6)
                .map(|item| item.id("sched.device"))
                .transpose()?,
            unknown: fields.unknown(),
        })
    }
}

/// A `sched`'s `idempotency_key`: `bstr .size (1..32)`.
fn idempotency_key(item: &Item<'_>) -> Result<Vec<u8>, DecodeError> {
    const FIELD: &str = "sched.idempotency_key";
    let key = item.bytes(FIELD)?;
    if (1..=IDEMPOTENCY_KEY_MAX).contains(&key.len()) {
        Ok(key.to_vec())
    } else {
        Err(field_error(
            FIELD,
            FieldProblem::Size {
                min: 1,
                max: IDEMPOTENCY_KEY_MAX,
                actual: key.len(),
            },
        ))
    }
}

impl Codec for TaskBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_id(1, &self.sender)
            .put_id(2, &self.channel)
            .put_id(3, &self.queue)
            .put_id(4, &self.task)
            .put(5, self.sched.encode())
            .put(6, self.mode.encode())
            .put(7, cbor::uint(self.enqueued))
            .put_bytes(8, &self.content.encode())
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("task-body")?;
        Ok(Self {
            sender: fields
                .required(1, "task-body.sender")?
                .id("task-body.sender")?,
            channel: fields
                .required(2, "task-body.channel")?
                .id("task-body.channel")?,
            queue: fields
                .required(3, "task-body.queue")?
                .id("task-body.queue")?,
            task: fields.required(4, "task-body.task")?.id("task-body.task")?,
            sched: Sched::decode(fields.required(5, "task-body.sched")?)?,
            mode: QueueMode::decode(fields.required(6, "task-body.mode")?)?,
            enqueued: fields
                .required(7, "task-body.enqueued")?
                .uint("task-body.enqueued")?,
            content: fields
                .required(8, "task-body.content")?
                .embedded("task-body.content")?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for QueueMode {
    fn encode(&self) -> Vec<u8> {
        cbor::uint(match self {
            Self::AtLeastOnce => 1,
            Self::AtMostOnce => 2,
        })
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        const FIELD: &str = "queue-mode";
        match item.uint(FIELD)? {
            1 => Ok(Self::AtLeastOnce),
            2 => Ok(Self::AtMostOnce),
            other => Err(field_error(FIELD, FieldProblem::Value(other))),
        }
    }
}

impl Codec for Task {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put(1, cbor::text(&self.request))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("task")?;
        Ok(Self {
            request: fields
                .required(1, "task.request")?
                .text("task.request")?
                .to_owned(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for TaskResult {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_id(1, &self.task)
            .put_id(2, &self.claim_id)
            .put(3, cbor::uint(self.generation))
            .put(4, self.outcome.encode())
            .put(5, cbor::text(&self.response))
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("task-result")?;
        Ok(Self {
            task: fields
                .required(1, "task-result.task")?
                .id("task-result.task")?,
            claim_id: fields
                .required(2, "task-result.claim_id")?
                .id("task-result.claim_id")?,
            generation: at_least(
                fields.required(3, "task-result.generation")?,
                "task-result.generation",
                1,
            )?,
            outcome: Outcome::decode(fields.required(4, "task-result.outcome")?)?,
            response: fields
                .required(5, "task-result.response")?
                .text("task-result.response")?
                .to_owned(),
            unknown: fields.unknown(),
        })
    }
}

impl Codec for Outcome {
    fn encode(&self) -> Vec<u8> {
        cbor::uint(match self {
            Self::Done => 1,
            Self::Failed => 2,
        })
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        const FIELD: &str = "outcome";
        match item.uint(FIELD)? {
            1 => Ok(Self::Done),
            2 => Ok(Self::Failed),
            other => Err(field_error(FIELD, FieldProblem::Value(other))),
        }
    }
}

impl Codec for Claim {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_id(1, &self.claim_id)
            .put(2, cbor::uint(self.generation))
            .put(3, cbor::uint(self.attempt))
            .put(4, cbor::uint(self.lease))
            .put(5, self.mode.encode())
            .put_id(6, &self.uploader)
            .put(7, self.task.encode())
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("claim")?;
        Ok(Self {
            claim_id: fields.required(1, "claim.claim_id")?.id("claim.claim_id")?,
            generation: at_least(
                fields.required(2, "claim.generation")?,
                "claim.generation",
                1,
            )?,
            attempt: at_least(fields.required(3, "claim.attempt")?, "claim.attempt", 1)?,
            lease: fields.required(4, "claim.lease")?.uint("claim.lease")?,
            mode: QueueMode::decode(fields.required(5, "claim.mode")?)?,
            uploader: fields.required(6, "claim.uploader")?.id("claim.uploader")?,
            task: QueuedTask::decode(fields.required(7, "claim.task")?)?,
            unknown: fields.unknown(),
        })
    }
}

impl Codec for QueueKeyBody {
    fn encode(&self) -> Vec<u8> {
        MapOut::new(&self.unknown)
            .put_id(1, &self.channel)
            .put(2, cbor::uint(self.epoch))
            .put_bytes(3, &self.key)
            .put_id(4, &self.signer)
            .finish()
    }

    fn decode(item: &Item<'_>) -> Result<Self, DecodeError> {
        let mut fields = item.map("queue-key-body")?;
        Ok(Self {
            channel: fields
                .required(1, "queue-key-body.channel")?
                .id("queue-key-body.channel")?,
            epoch: at_least(
                fields.required(2, "queue-key-body.epoch")?,
                "queue-key-body.epoch",
                1,
            )?,
            key: fields
                .required(3, "queue-key-body.key")?
                .fixed("queue-key-body.key")?,
            signer: fields
                .required(4, "queue-key-body.signer")?
                .id("queue-key-body.signer")?,
            unknown: fields.unknown(),
        })
    }
}

cbor::cbor!(
    QueuedTask,
    Sched,
    TaskBody,
    QueueMode,
    Task,
    TaskResult,
    Outcome,
    Claim,
    QueueKeyBody
);
