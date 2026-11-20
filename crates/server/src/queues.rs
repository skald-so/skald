//! A channel's queues and their tasks on the server: `docs/protocol/queues.md`, "Queues", "Tasks"
//! and "Task states". Each request proves its device as `/mailbox/send` does, a `service` device's
//! with its token, so a session or a token alone makes none. Every channel has `default`, which the
//! server creates with the channel's ID and nobody deletes. A `client` device of a member of the
//! channel creates a queue; one of its creator's account or of an admin changes a setting other than
//! `mode`, or deletes it with its tasks. The server stores a task, as the bytes it got, only from a
//! device that may send in the channel and, a `service` device, whose token has `queues`; once per
//! task ID and per `idempotency_key` in its queue, and answers a device's repeated upload of a task
//! ID as it answered the first. It gives tasks only to devices that may send in the channel.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use skald_proto::{
    AccountId, Cbor as _, ChannelId, DeviceId, Name, QueueId, QueuedTask, SpaceId, TaskId,
};
use sqlx::{PgConnection, PgPool};

use crate::devices::base64;
use crate::mailboxes::{latest, listed};
use crate::refusal::{Refusal, parse};
use crate::requests::{self, InChannel, invalid};
use crate::spaces::admin;
use crate::{AppState, database_failed, problem};

/// The queue every channel has.
const DEFAULT: &str = "default";
/// The default mode, lease in seconds and attempts of a queue, and the bounds of the last two.
const MODE: Mode = Mode::AtLeastOnce;
const LEASE: i32 = 300;
const LEASES: std::ops::RangeInclusive<i32> = 30..=3600;
const MAX_ATTEMPTS: i32 = 5;
const ATTEMPTS: std::ops::RangeInclusive<i32> = 1..=100;
/// The longest body of an enqueue.
const TASK_BODY_LIMIT: usize = 64 << 20;
/// The longest body of a queue's settings.
const SETTINGS_BODY_LIMIT: usize = 4096;

/// A queue's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Mode {
    AtLeastOnce,
    AtMostOnce,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Mode::AtLeastOnce => "at-least-once",
            Mode::AtMostOnce => "at-most-once",
        }
    }

    fn of(stored: &str) -> Self {
        match stored {
            "at-most-once" => Mode::AtMostOnce,
            _ => Mode::AtLeastOnce,
        }
    }
}

/// `Queue` in the spec.
#[derive(Serialize)]
struct Queue {
    id: String,
    name: String,
    mode: Mode,
    lease: i32,
    max_attempts: i32,
}

/// `Queues` in the spec.
#[derive(Serialize)]
struct Queues {
    queues: Vec<Queue>,
}

/// `QueueRequest` and `QueueChange` in the spec: the settings a request states.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    name: Option<String>,
    mode: Option<Mode>,
    lease: Option<i32>,
    max_attempts: Option<i32>,
}

impl Settings {
    /// Refuses a setting out of range or a name that is no `name`.
    fn check(&self) -> Result<(), Box<Response>> {
        let refused = |detail| Err(Box::new(invalid(detail)));
        if let Some(name) = &self.name
            && name.parse::<Name>().is_err()
        {
            return refused("The name is no name of an agent address.");
        }
        if self.lease.is_some_and(|lease| !LEASES.contains(&lease)) {
            return refused("The lease is not 30 to 3600 seconds.");
        }
        if (self.max_attempts).is_some_and(|attempts| !ATTEMPTS.contains(&attempts)) {
            return refused("max_attempts is not 1 to 100.");
        }
        Ok(())
    }
}

/// `Task` in the spec.
#[derive(Serialize)]
struct Task {
    queued_task: String,
    state: String,
    attempts: i32,
}

/// `Tasks` in the spec.
#[derive(Serialize)]
struct Tasks {
    tasks: Vec<Task>,
}

/// A queue as the server keeps it, with the account that created it.
struct Kept {
    queue: Queue,
    creator: Option<AccountId>,
}

/// Creates `default` of `channel` on `conn`, as the server issues the channel's ID.
pub(crate) async fn create_default(
    conn: &mut PgConnection,
    channel: ChannelId,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO queues (id, channel_id, name, mode, lease, max_attempts)
         VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)",
    )
    .bind(QueueId::generate().to_string())
    .bind(channel.to_string())
    .bind(DEFAULT)
    .bind(MODE.as_str())
    .bind(LEASE)
    .bind(MAX_ATTEMPTS)
    .execute(conn)
    .await?;
    Ok(())
}

/// `GET /spaces/<space>/channels/<channel>/queues`: the channel's queues, for a device that may send
/// in it.
pub(crate) async fn list(State(state): State<AppState>, at: InChannel) -> Response {
    match of_channel(&state.db, &at).await {
        Ok(Some(queues)) => Json(Queues { queues }).into_response(),
        Ok(None) => not_a_member(),
        Err(err) => database_failed("read a channel's queues", err),
    }
}

/// The queues of `at`'s channel, by name, if its device may send in it.
async fn of_channel(db: &PgPool, at: &InChannel) -> Result<Option<Vec<Queue>>, sqlx::Error> {
    if !at.sends(db).await? {
        return Ok(None);
    }
    let found: Vec<Row> = sqlx::query_as(
        "SELECT q.id::text, q.name, q.mode, q.lease, q.max_attempts, q.creator_id::text
         FROM queues q JOIN channels c ON c.id = q.channel_id
         WHERE q.channel_id = $1::uuid AND c.space_id = $2::uuid ORDER BY q.name",
    )
    .bind(at.channel.to_string())
    .bind(at.space.to_string())
    .fetch_all(db)
    .await?;
    Ok(Some(found.into_iter().map(|row| kept(row).queue).collect()))
}

/// `POST /spaces/<space>/channels/<channel>/queues`: a new queue of the channel with the body's
/// settings and the defaults for the others, from a `client` device in the channel.
pub(crate) async fn create(State(state): State<AppState>, at: InChannel, body: Body) -> Response {
    let settings = match settings(body).await {
        Ok(settings) => settings,
        Err(refused) => return *refused,
    };
    let Some(name) = &settings.name else {
        return invalid("The body has no name.");
    };
    if let Err(refused) = settings.check() {
        return *refused;
    }
    // Without a token, a device may send in the channel if it is in it.
    let in_it = match at.proved.token {
        Some(_) => false,
        None => match at.sends(&state.db).await {
            Ok(sends) => sends,
            Err(err) => return database_failed("read a revision", err),
        },
    };
    if !in_it {
        return not_allowed("Only a client device in the channel creates a queue in it.");
    }
    let queue = Queue {
        id: QueueId::generate().to_string(),
        name: name.clone(),
        mode: settings.mode.unwrap_or(MODE),
        lease: settings.lease.unwrap_or(LEASE),
        max_attempts: settings.max_attempts.unwrap_or(MAX_ATTEMPTS),
    };
    let inserted = sqlx::query(
        "INSERT INTO queues (id, channel_id, name, mode, lease, max_attempts, creator_id)
         SELECT $1::uuid, c.id, $3, $4, $5, $6, $7::uuid FROM channels c
         WHERE c.id = $2::uuid AND c.space_id = $8::uuid
         ON CONFLICT (channel_id, name) DO NOTHING",
    )
    .bind(&queue.id)
    .bind(at.channel.to_string())
    .bind(&queue.name)
    .bind(queue.mode.as_str())
    .bind(queue.lease)
    .bind(queue.max_attempts)
    .bind(at.proved.account.to_string())
    .bind(at.space.to_string())
    .execute(&state.db)
    .await;
    match inserted {
        Ok(done) if done.rows_affected() == 1 => (StatusCode::CREATED, Json(queue)).into_response(),
        // The revision lists only channels whose IDs the server issued for its space: the name is
        // taken.
        Ok(_) => name_taken(),
        Err(err) => database_failed("create a queue", err),
    }
}

/// `PATCH /spaces/<space>/channels/<channel>/queues/<queue>`: changes the settings the body states,
/// other than `mode`, from a `client` device of the queue's creator or of an admin; `default` keeps
/// its name.
pub(crate) async fn change(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue)): Path<(String, String, String)>,
    body: Body,
) -> Response {
    let kept = match changer(&state, &at, &queue).await {
        Ok(found) => found,
        Err(refused) => return *refused,
    };
    let settings = match settings(body).await {
        Ok(settings) => settings,
        Err(refused) => return *refused,
    };
    if settings.mode.is_some() {
        return not_allowed("A queue's mode is fixed at creation.");
    }
    if settings.name.is_some() && kept.queue.name == DEFAULT {
        return not_allowed("default keeps its name.");
    }
    if let Err(refused) = settings.check() {
        return *refused;
    }
    let queue = Queue {
        name: settings.name.unwrap_or(kept.queue.name),
        lease: settings.lease.unwrap_or(kept.queue.lease),
        max_attempts: settings.max_attempts.unwrap_or(kept.queue.max_attempts),
        ..kept.queue
    };
    let changed = sqlx::query(
        "UPDATE queues SET name = $2, lease = $3, max_attempts = $4 WHERE id = $1::uuid",
    )
    .bind(&queue.id)
    .bind(&queue.name)
    .bind(queue.lease)
    .bind(queue.max_attempts)
    .execute(&state.db)
    .await;
    match changed {
        Ok(done) if done.rows_affected() == 1 => Json(queue).into_response(),
        Ok(_) => unknown_queue(),
        Err(err)
            if err
                .as_database_error()
                .is_some_and(|e| e.is_unique_violation()) =>
        {
            name_taken()
        }
        Err(err) => database_failed("change a queue", err),
    }
}

/// `DELETE /spaces/<space>/channels/<channel>/queues/<queue>`: deletes the queue with its tasks,
/// from a `client` device of its creator or of an admin; nobody deletes `default`.
pub(crate) async fn delete(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue)): Path<(String, String, String)>,
) -> Response {
    let kept = match changer(&state, &at, &queue).await {
        Ok(found) => found,
        Err(refused) => return *refused,
    };
    if kept.queue.name == DEFAULT {
        return not_allowed("Nobody deletes default.");
    }
    let deleted = sqlx::query("DELETE FROM queues WHERE id = $1::uuid")
        .bind(&kept.queue.id)
        .execute(&state.db)
        .await;
    match deleted {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => database_failed("delete a queue", err),
    }
}

/// The queue `queue` of `at`'s channel, if `at`'s device may change it: a `client` device listed in
/// the space's latest revision, of the queue's creator or of an admin; the refusal otherwise.
async fn changer(state: &AppState, at: &InChannel, queue: &str) -> Result<Kept, Box<Response>> {
    let InChannel {
        proved,
        space,
        channel,
    } = at;
    let refused = || {
        Box::new(not_allowed(
            "Only a client device of the queue's creator or of an admin changes or deletes it.",
        ))
    };
    let roster = (latest(&state.db, *space).await)
        .map_err(|err| Box::new(database_failed("read a revision", err)))?;
    let Some(roster) = roster.filter(|roster| listed(roster.body(), proved.account, proved.device))
    else {
        return Err(refused());
    };
    if proved.token.is_some() {
        return Err(refused());
    }
    let Some(queue) = queue.parse().ok() else {
        return Err(Box::new(unknown_queue()));
    };
    let kept = (find(&state.db, *space, *channel, queue).await)
        .map_err(|err| Box::new(database_failed("read a queue", err)))?
        .ok_or_else(|| Box::new(unknown_queue()))?;
    let allowed = admin(&roster, proved.account) || kept.creator == Some(proved.account);
    if !allowed {
        return Err(refused());
    }
    Ok(kept)
}

/// `POST /spaces/<space>/channels/<channel>/queues/<queue>/tasks`: stores the body, a
/// `queued-task`, as a task of the queue, or refuses it; a device's repeated upload of a task ID
/// gets the first answer.
pub(crate) async fn enqueue(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue)): Path<(String, String, String)>,
    body: Body,
) -> Response {
    let Ok(body) = axum::body::to_bytes(body, TASK_BODY_LIMIT).await else {
        return Upload::Refused(Problem::TooLarge).response();
    };
    let Ok(task) = QueuedTask::from_cbor(&body) else {
        return invalid("The body is not a queued-task.");
    };
    let upload = Uploaded {
        device: at.proved.device,
        task: task.task,
    };
    loop {
        match answered(&state.db, &upload).await {
            Ok(Some(first)) => return first.response(),
            Ok(None) => {}
            Err(err) => return database_failed("read a task's upload", err),
        }
        match store(&state, &at, &queue, &task, &body).await {
            Ok(Ok(Some(answer))) => return answer.response(),
            // A retry from the same device was answered meanwhile: its answer is read again.
            Ok(Ok(None)) => {}
            Ok(Err(refused)) => return *refused,
            Err(err) => return database_failed("enqueue a task", err),
        }
    }
}

/// A device's upload of a task ID.
struct Uploaded {
    device: DeviceId,
    task: TaskId,
}

/// The answer to an upload: stored, or refused for a problem of `queues.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Upload {
    Stored,
    Refused(Problem),
}

/// Why the server refuses an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    NotAMember,
    UnknownQueue,
    /// The task stored before.
    Duplicate(TaskId),
    EpochMismatch,
    TooLarge,
}

impl Problem {
    fn name(self) -> &'static str {
        match self {
            Problem::NotAMember => "not-a-member",
            Problem::UnknownQueue => "unknown-queue",
            Problem::Duplicate(_) => "duplicate",
            Problem::EpochMismatch => "epoch-mismatch",
            Problem::TooLarge => "too-large",
        }
    }

    fn of(name: &str, duplicate: Option<TaskId>) -> Option<Self> {
        Some(match name {
            "not-a-member" => Problem::NotAMember,
            "unknown-queue" => Problem::UnknownQueue,
            "duplicate" => Problem::Duplicate(duplicate?),
            "epoch-mismatch" => Problem::EpochMismatch,
            "too-large" => Problem::TooLarge,
            _ => return None,
        })
    }
}

impl Upload {
    fn response(self) -> Response {
        let problem = match self {
            Upload::Stored => return StatusCode::NO_CONTENT.into_response(),
            Upload::Refused(problem) => problem,
        };
        match problem {
            Problem::NotAMember => not_a_member(),
            Problem::UnknownQueue => unknown_queue(),
            Problem::Duplicate(task) => duplicate(task),
            Problem::EpochMismatch => crate::problem(
                StatusCode::CONFLICT,
                "epoch-mismatch",
                "Another epoch",
                "The task is sealed under an epoch other than the channel's current or previous one.",
            ),
            Problem::TooLarge => crate::problem(
                StatusCode::PAYLOAD_TOO_LARGE,
                "too-large",
                "Task too large",
                "The task's ct is over the server's limit.",
            ),
        }
    }
}

/// The answer `upload` got first, if the server stored or refused it.
async fn answered(db: &PgPool, upload: &Uploaded) -> Result<Option<Upload>, sqlx::Error> {
    let found: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT problem, duplicate::text FROM task_uploads
         WHERE device_id = $1::uuid AND task_id = $2::uuid",
    )
    .bind(upload.device.to_string())
    .bind(upload.task.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(|(problem, duplicate)| match problem {
        None => Upload::Stored,
        Some(name) => {
            let duplicate = duplicate.map(|task| task.parse().expect("a stored task ID"));
            Upload::Refused(Problem::of(&name, duplicate).expect("a stored problem"))
        }
    }))
}

/// Keeps `answer` as the answer to `upload` on `conn`; whether it is the first.
async fn keep(
    conn: &mut PgConnection,
    upload: &Uploaded,
    answer: Upload,
) -> Result<bool, sqlx::Error> {
    let (problem, duplicate) = match answer {
        Upload::Stored => (None, None),
        Upload::Refused(problem) => (
            Some(problem.name()),
            match problem {
                Problem::Duplicate(task) => Some(task.to_string()),
                _ => None,
            },
        ),
    };
    // Of two inserts at once, the second waits for the first to commit, and inserts nothing.
    let kept = sqlx::query(
        "INSERT INTO task_uploads (device_id, task_id, problem, duplicate)
         VALUES ($1::uuid, $2::uuid, $3, $4::uuid) ON CONFLICT DO NOTHING",
    )
    .bind(upload.device.to_string())
    .bind(upload.task.to_string())
    .bind(problem)
    .bind(duplicate)
    .execute(conn)
    .await?;
    Ok(kept.rows_affected() == 1)
}

/// Stores `task`, of `bytes`, from `at`'s device in the queue `queue` of its channel, or refuses
/// it; gives the answer, kept for a retry, or `None` if an upload of the task from the same device
/// was answered meanwhile. A request whose task names another queue is refused and not kept.
async fn store(
    state: &AppState,
    at: &InChannel,
    queue: &str,
    task: &QueuedTask,
    bytes: &[u8],
) -> Result<Result<Option<Upload>, Box<Response>>, sqlx::Error> {
    let db = &state.db;
    let device = at.proved.device;
    let upload = Uploaded {
        device,
        task: task.task,
    };
    let checked = check(state, at, queue, task).await?;
    let mut tx = db.begin().await?;
    let answer = match checked {
        Ok(Some(problem)) => Upload::Refused(problem),
        Ok(None) => match insert(&mut tx, device, task, bytes).await? {
            Some(answer) => answer,
            None => return Ok(Ok(None)),
        },
        Err(refused) => return Ok(Err(refused)),
    };
    if !keep(&mut tx, &upload, answer).await? {
        return Ok(Ok(None));
    }
    tx.commit().await?;
    Ok(Ok(Some(answer)))
}

/// The problem `task`'s upload to the queue `queue` of `at`'s channel from its device has before it
/// is stored, if any; the refusal of a task that names another queue.
async fn check(
    state: &AppState,
    at: &InChannel,
    queue: &str,
    task: &QueuedTask,
) -> Result<Result<Option<Problem>, Box<Response>>, sqlx::Error> {
    let db = &state.db;
    let (space, channel) = (at.space, at.channel);
    let allowed = match &at.proved.token {
        Some(token) => token.enqueues_in(space, channel),
        None => at.sends(db).await?,
    };
    if !allowed {
        return Ok(Ok(Some(Problem::NotAMember)));
    }
    let Ok(queue) = queue.parse::<QueueId>() else {
        return Ok(Ok(Some(Problem::UnknownQueue)));
    };
    if task.queue != queue {
        return Ok(Err(Box::new(invalid("The task names another queue."))));
    }
    if find(db, space, channel, queue).await?.is_none() {
        return Ok(Ok(Some(Problem::UnknownQueue)));
    }
    let current: Option<(i64,)> =
        sqlx::query_as("SELECT epoch FROM channel_groups WHERE channel_id = $1::uuid")
            .bind(channel.to_string())
            .fetch_optional(db)
            .await?;
    let current = current.map_or(0, |(epoch,)| u64::try_from(epoch).unwrap_or(0));
    // A channel without a group has no epoch 0 to match.
    if task.epoch != current && task.epoch.checked_add(1) != Some(current) {
        return Ok(Ok(Some(Problem::EpochMismatch)));
    }
    let limit = state.limits.message_size;
    if limit.is_some_and(|limit| task.ct.len() as u64 > limit) {
        return Ok(Ok(Some(Problem::TooLarge)));
    }
    Ok(Ok(None))
}

/// Inserts `task`, of `bytes`, from `device` on `tx`, `queued` with attempts 0: gives `stored`, or
/// `duplicate` naming the task with its ID from another device or with its `idempotency_key` in
/// its queue, or `unknown-queue` if the queue is gone. `None` if `device` uploaded the task ID
/// meanwhile.
async fn insert(
    tx: &mut PgConnection,
    device: DeviceId,
    task: &QueuedTask,
    bytes: &[u8],
) -> Result<Option<Upload>, sqlx::Error> {
    // The queue's row lock keeps its deletion from passing the insert.
    let queue: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM queues WHERE id = $1::uuid FOR SHARE")
            .bind(task.queue.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if queue.is_none() {
        return Ok(Some(Upload::Refused(Problem::UnknownQueue)));
    }
    let key = task.sched.idempotency_key.as_deref();
    // Of two inserts of the same task ID or key at once, the second waits for the first to commit,
    // and inserts nothing.
    let inserted = sqlx::query(
        "INSERT INTO tasks (id, queue_id, uploader_id, queued_task, idempotency_key)
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5) ON CONFLICT DO NOTHING",
    )
    .bind(task.task.to_string())
    .bind(task.queue.to_string())
    .bind(device.to_string())
    .bind(bytes)
    .bind(key)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 1 {
        return Ok(Some(Upload::Stored));
    }
    let same_id: Option<(String,)> =
        sqlx::query_as("SELECT uploader_id::text FROM tasks WHERE id = $1::uuid")
            .bind(task.task.to_string())
            .fetch_optional(&mut *tx)
            .await?;
    if let Some((uploader,)) = same_id {
        if uploader == device.to_string() {
            return Ok(None);
        }
        return Ok(Some(Upload::Refused(Problem::Duplicate(task.task))));
    }
    let same_key: Option<(String,)> = sqlx::query_as(
        "SELECT id::text FROM tasks WHERE queue_id = $1::uuid AND idempotency_key = $2",
    )
    .bind(task.queue.to_string())
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?;
    // Without it, the task it met is gone meanwhile: the upload is tried again.
    Ok(same_key.map(|(other,)| {
        Upload::Refused(Problem::Duplicate(other.parse().expect("a stored task ID")))
    }))
}

/// `GET /spaces/<space>/channels/<channel>/queues/<queue>/tasks`: the queue's tasks in the order the
/// server stored them, for a device that may send in the channel.
pub(crate) async fn tasks(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue)): Path<(String, String, String)>,
) -> Response {
    match given(&state, &at, &queue, None).await {
        Ok(tasks) => Json(Tasks { tasks }).into_response(),
        Err(refused) => *refused,
    }
}

/// `GET /spaces/<space>/channels/<channel>/queues/<queue>/tasks/<task>`: the task, for a device that
/// may send in the channel.
pub(crate) async fn task(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue, task)): Path<(String, String, String, String)>,
) -> Response {
    let Ok(task) = task.parse::<TaskId>() else {
        return unknown_task();
    };
    match given(&state, &at, &queue, Some(task)).await {
        Ok(mut tasks) => match tasks.pop() {
            Some(task) => Json(task).into_response(),
            None => unknown_task(),
        },
        Err(refused) => *refused,
    }
}

/// The tasks of the queue `queue` of `at`'s channel, or only `task` of them, if `at`'s device may
/// send in the channel; the refusal otherwise.
async fn given(
    state: &AppState,
    at: &InChannel,
    queue: &str,
    task: Option<TaskId>,
) -> Result<Vec<Task>, Box<Response>> {
    let failed = |err| Box::new(database_failed("read a queue's tasks", err));
    if !at.sends(&state.db).await.map_err(failed)? {
        return Err(Box::new(not_a_member()));
    }
    let Some(queue) = queue.parse().ok() else {
        return Err(Box::new(unknown_queue()));
    };
    if find(&state.db, at.space, at.channel, queue)
        .await
        .map_err(failed)?
        .is_none()
    {
        return Err(Box::new(unknown_queue()));
    }
    let found: Vec<(Vec<u8>, String, i32)> = sqlx::query_as(
        "SELECT queued_task, state, attempts FROM tasks
         WHERE queue_id = $1::uuid AND ($2::uuid IS NULL OR id = $2::uuid) ORDER BY seq",
    )
    .bind(queue.to_string())
    .bind(task.map(|task| task.to_string()))
    .fetch_all(&state.db)
    .await
    .map_err(failed)?;
    Ok(found
        .into_iter()
        .map(|(bytes, state, attempts)| Task {
            queued_task: base64(&bytes),
            state,
            attempts,
        })
        .collect())
}

/// A queue's row as the queries read it.
type Row = (String, String, String, i32, i32, Option<String>);

fn kept((id, name, mode, lease, max_attempts, creator): Row) -> Kept {
    Kept {
        queue: Queue {
            id,
            name,
            mode: Mode::of(&mode),
            lease,
            max_attempts,
        },
        creator: creator.map(|account| account.parse().expect("a stored account ID")),
    }
}

/// The queue `queue` of `channel` of `space`, if it has it.
async fn find(
    db: &PgPool,
    space: SpaceId,
    channel: ChannelId,
    queue: QueueId,
) -> Result<Option<Kept>, sqlx::Error> {
    let found: Option<Row> = sqlx::query_as(
        "SELECT q.id::text, q.name, q.mode, q.lease, q.max_attempts, q.creator_id::text
         FROM queues q JOIN channels c ON c.id = q.channel_id
         WHERE q.id = $1::uuid AND q.channel_id = $2::uuid AND c.space_id = $3::uuid",
    )
    .bind(queue.to_string())
    .bind(channel.to_string())
    .bind(space.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.map(kept))
}

/// The settings of the JSON `body`; the refusal of a body that is none.
async fn settings(body: Body) -> Result<Settings, Box<Response>> {
    let Ok(body) = axum::body::to_bytes(body, SETTINGS_BODY_LIMIT).await else {
        return Err(Box::new(invalid("The body is over 4096 bytes.")));
    };
    parse(&body).map_err(|refusal: Refusal| Box::new(refusal.into_response()))
}

fn not_a_member() -> Response {
    requests::not_a_member(
        "The device may not send in the channel, or its token has no queues permission.",
    )
}

fn not_allowed(detail: &str) -> Response {
    problem(StatusCode::FORBIDDEN, "not-allowed", "Not allowed", detail)
}

fn unknown_queue() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "unknown-queue",
        "No such queue",
        "The channel has no such queue.",
    )
}

fn unknown_task() -> Response {
    problem(
        StatusCode::NOT_FOUND,
        "unknown-task",
        "No such task",
        "The queue has no such task.",
    )
}

fn name_taken() -> Response {
    problem(
        StatusCode::CONFLICT,
        "queue-name-taken",
        "Queue name taken",
        "The channel has a queue of this name.",
    )
}

/// `DuplicateProblem` in the spec, naming `task`.
fn duplicate(task: TaskId) -> Response {
    crate::problem_details(
        StatusCode::CONFLICT,
        serde_json::json!({
            "type": "https://skald.so/problems/duplicate",
            "title": "Duplicate task",
            "status": StatusCode::CONFLICT.as_u16(),
            "detail": "Another device used the task ID, or another task of the queue has the idempotency_key.",
            "task": task.to_string(),
        }),
    )
}
