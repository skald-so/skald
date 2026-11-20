//! Claims of a queue's tasks and what follows them: `docs/protocol/queues.md`, "Task states",
//! "Claims and leases", "Fencing", "Modes and retries" and "Claim checks". Only a device that
//! passes the server's claim checks claims a task, or extends, completes, fails or releases its
//! claim. A claim gets the first task in storing order that is `queued`, past its backoff, before
//! its `deadline`, not in the claimer's wait after its release, and that matches it, locked with
//! `SKIP LOCKED` so that claims at once take different tasks. Extend, complete, fail and release
//! pass the fencing check in the transaction that applies them; the server keeps the act that ended
//! each claim, which a repeat of it gets again. A `client` or `browser` device of the uploader's
//! account or of an admin restarts or cancels a task. Every instance applies lease ends and passed
//! deadlines, and deletes finished tasks past the history retention, as they come due.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use skald_proto::{
    Cbor as _, Claim, ClaimId, Name, QueueId, QueueMode, QueuedTask, TaskId, Unknown,
};
use sqlx::{PgConnection, PgPool};

use crate::mailboxes::{latest, listed};
use crate::queues::{Mode, find, json, not_allowed, unknown_queue, unknown_task};
use crate::requests::{self, InChannel, invalid};
use crate::spaces::admin;
use crate::sweeps::{self, PAGE, PAUSE};
use crate::{AppState, database_failed, problem};

/// The release since enqueue or the last restart that makes a task `dead`.
const RELEASES: i32 = 5;
/// The first backoff and release wait, in seconds, and the longest backoff.
const FIRST_WAIT: i64 = 10;
const LONGEST_BACKOFF: i64 = 3600;

/// A sender's trust level, as a claim states it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Level {
    Own,
    Peer,
    Service,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Own => "own",
            Level::Peer => "peer",
            Level::Service => "service",
        }
    }
}

/// `ClaimRequest` in the spec: what a claim takes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Takes {
    trust_levels: Vec<Level>,
    templates: Vec<String>,
    without_template: bool,
}

/// The progress an extend states.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Progress {
    Started,
    AwaitingApproval,
}

impl Progress {
    fn as_str(self) -> &'static str {
        match self {
            Progress::Started => "started",
            Progress::AwaitingApproval => "awaiting-approval",
        }
    }
}

/// `ClaimReference`, `ExtendRequest` and `FailRequest` in the spec: the claim an act names, with
/// the progress of an extend and whether a fail retries.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    claim_id: String,
    generation: u64,
    progress: Option<Progress>,
    retry: Option<bool>,
}

/// The acts on a claim, as the routes name them.
pub(crate) const EXTEND: u8 = 0;
pub(crate) const COMPLETE: u8 = 1;
pub(crate) const FAIL: u8 = 2;
pub(crate) const RELEASE: u8 = 3;

/// The changes a human makes, as the routes name them.
pub(crate) const RESTART: u8 = 0;
pub(crate) const CANCEL: u8 = 1;

/// What a holder does with its claim.
#[derive(Debug, Clone, Copy)]
enum Act {
    Extend(Progress),
    Complete,
    Fail { retry: bool },
    Release,
}

impl Act {
    /// The name of an act that ends the claim, as the server keeps it; `None` for an extend.
    fn ending(self) -> Option<&'static str> {
        match self {
            Act::Extend(_) => None,
            Act::Complete => Some("complete"),
            Act::Fail { .. } => Some("fail"),
            Act::Release => Some("release"),
        }
    }
}

/// What a human does with a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Restart,
    Cancel,
}

/// A path to a task: the space, the channel, the queue and the task.
type TaskPath = Path<(String, String, String, String)>;

/// `POST /spaces/<space>/channels/<channel>/queues/<queue>/claims`: the `claim` of the first task
/// that matches the body, or `204` when none does.
pub(crate) async fn claim(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue)): Path<(String, String, String)>,
    body: Body,
) -> Response {
    let takes = match json::<Takes>(body).await {
        Ok(takes) => takes,
        Err(refused) => return *refused,
    };
    if (takes.templates.iter()).any(|template| template.parse::<Name>().is_err()) {
        return invalid("A template is no name of an agent address.");
    }
    match claimed(&state.db, &at, &queue, &takes).await {
        Ok(Ok(Some(claim))) => (
            [(header::CONTENT_TYPE, "application/cbor")],
            claim.to_cbor(),
        )
            .into_response(),
        Ok(Ok(None)) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(refused)) => *refused,
        Err(err) => database_failed("claim a task", err),
    }
}

/// Claims for `at`'s device the first task of the queue `queue` that matches `takes`, if any; the
/// refusal of a device that fails the claim checks or of an unknown queue.
async fn claimed(
    db: &PgPool,
    at: &InChannel,
    queue: &str,
    takes: &Takes,
) -> Result<Result<Option<Claim>, Box<Response>>, sqlx::Error> {
    if !claims_in(db, at).await? {
        return Ok(Err(Box::new(not_a_member())));
    }
    let Some(queue) = queue_of(db, at, queue).await? else {
        return Ok(Err(Box::new(unknown_queue())));
    };
    // Another account's device is `peer` while the roster lists its account as a member.
    let members: Vec<String> = (latest(db, at.space).await?)
        .map(|roster| {
            (roster.body().members.iter())
                .map(|member| member.account.to_string())
                .collect()
        })
        .unwrap_or_default();
    let levels: Vec<&str> = takes
        .trust_levels
        .iter()
        .map(|level| level.as_str())
        .collect();
    let device = at.proved.device.to_string();
    let mut tx = db.begin().await?;
    let found: Option<(String, Vec<u8>, String)> = sqlx::query_as(
        "SELECT t.id::text, t.queued_task, t.uploader_id::text
         FROM tasks t JOIN devices u ON u.id = t.uploader_id
         WHERE t.queue_id = $1::uuid AND t.state = 'queued'
           AND (t.not_before IS NULL OR t.not_before <= now())
           AND (t.deadline IS NULL OR t.deadline > now())
           AND NOT EXISTS (
               SELECT 1 FROM task_waits w
               WHERE w.task_id = t.id AND w.device_id = $2::uuid AND w.until > now())
           AND (t.addressee IS NULL OR t.addressee = $2::uuid)
           AND CASE WHEN u.kind = 'service' THEN 'service'
                    WHEN u.account_id = $3::uuid THEN 'own'
                    WHEN u.account_id = ANY($4::text[]::uuid[]) THEN 'peer'
               END = ANY($5::text[])
           AND (t.template = ANY($6::text[]) OR (t.template IS NULL AND $7))
         ORDER BY t.seq LIMIT 1
         FOR UPDATE OF t SKIP LOCKED",
    )
    .bind(queue.to_string())
    .bind(&device)
    .bind(at.proved.account.to_string())
    .bind(&members)
    .bind(&levels)
    .bind(&takes.templates)
    .bind(takes.without_template)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((task, bytes, uploader)) = found else {
        return Ok(Ok(None));
    };
    let claim_id = ClaimId::generate();
    let (generation, attempts, lease, mode): (i64, i32, i32, String) = sqlx::query_as(
        "UPDATE tasks t
         SET state = 'claimed', generation = t.generation + 1, attempts = t.attempts + 1,
             claim_id = $2::uuid, claim_device = $3::uuid, progress = NULL, reason = NULL,
             not_before = NULL, lease_until = now() + make_interval(secs => q.lease), since = now()
         FROM queues q WHERE t.id = $1::uuid AND q.id = t.queue_id
         RETURNING t.generation, t.attempts, q.lease, q.mode",
    )
    .bind(&task)
    .bind(claim_id.to_string())
    .bind(&device)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO claims (id, task_id, generation, device_id, lease)
         VALUES ($1::uuid, $2::uuid, $3, $4::uuid, $5)",
    )
    .bind(claim_id.to_string())
    .bind(&task)
    .bind(generation)
    .bind(&device)
    .bind(lease)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Ok(Some(Claim {
        claim_id,
        generation: u64::try_from(generation).expect("a generation from 1"),
        attempt: u64::try_from(attempts).expect("attempts from 1"),
        lease: u64::try_from(lease).expect("a lease from 30"),
        mode: match Mode::of(&mode) {
            Mode::AtLeastOnce => QueueMode::AtLeastOnce,
            Mode::AtMostOnce => QueueMode::AtMostOnce,
        },
        uploader: uploader.parse().expect("a stored device ID"),
        task: QueuedTask::from_cbor(&bytes).expect("a stored task decodes"),
        unknown: Unknown::default(),
    })))
}

/// `POST .../tasks/<task>/<act>`, of [`EXTEND`], [`COMPLETE`], [`FAIL`] or [`RELEASE`]: an extend
/// renews the claim's lease and keeps the progress it states; a complete makes the task `done`; a
/// fail makes it `queued` after its backoff, or `dead`; a release hands it back, `queued` at once
/// or, the 5th time, `dead`. `204`, or the refusal.
pub(crate) async fn act<const ACT: u8>(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue, task)): TaskPath,
    body: Body,
) -> Response {
    let named = match json::<Named>(body).await {
        Ok(named) => named,
        Err(refused) => return *refused,
    };
    let act = match (ACT, named.progress, named.retry) {
        (EXTEND, Some(progress), None) => Act::Extend(progress),
        (COMPLETE, None, None) => Act::Complete,
        (FAIL, None, Some(retry)) => Act::Fail { retry },
        (RELEASE, None, None) => Act::Release,
        _ => return invalid("The body lacks a member the act needs, or has one it does not take."),
    };
    match acted(&state.db, &at, &queue, &task, &named, act).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(refused)) => *refused,
        Err(err) => database_failed("take an act on a claim", err),
    }
}

/// A task as [`locked`] reads it: its state, last claim's ID, generation and device, whether the
/// lease lasts, its attempts and releases, its queue's mode and `max_attempts`, and its uploader's
/// account.
#[derive(sqlx::FromRow)]
struct Locked {
    state: String,
    claim_id: Option<String>,
    generation: i64,
    claim_device: Option<String>,
    lasts: bool,
    attempts: i32,
    releases: i32,
    mode: String,
    max_attempts: i32,
    uploader: String,
}

/// The task `task` of the queue `queue` of `at`'s channel, locked on `tx` until it commits; the
/// refusal of an unknown queue or task.
async fn locked(
    tx: &mut PgConnection,
    at: &InChannel,
    queue: &str,
    task: &str,
) -> Result<Result<(String, Locked), Box<Response>>, sqlx::Error> {
    let Ok(queue) = queue.parse::<QueueId>() else {
        return Ok(Err(Box::new(unknown_queue())));
    };
    if find(&mut *tx, at.space, at.channel, queue).await?.is_none() {
        return Ok(Err(Box::new(unknown_queue())));
    }
    let Ok(task) = task.parse::<TaskId>() else {
        return Ok(Err(Box::new(unknown_task())));
    };
    let found: Option<Locked> = sqlx::query_as(
        "SELECT t.state, t.claim_id::text AS claim_id, t.generation,
                t.claim_device::text AS claim_device,
                coalesce(t.lease_until > now(), false) AS lasts, t.attempts, t.releases, q.mode,
                q.max_attempts, u.account_id::text AS uploader
         FROM tasks t JOIN queues q ON q.id = t.queue_id JOIN devices u ON u.id = t.uploader_id
         WHERE t.id = $1::uuid AND t.queue_id = $2::uuid FOR UPDATE OF t",
    )
    .bind(task.to_string())
    .bind(queue.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    Ok(match found {
        Some(found) => Ok((task.to_string(), found)),
        None => Err(Box::new(unknown_task())),
    })
}

/// Takes `act` as [`act`] says: checks the device, then fences and applies it in one transaction.
async fn acted(
    db: &PgPool,
    at: &InChannel,
    queue: &str,
    task: &str,
    named: &Named,
    act: Act,
) -> Result<Result<(), Box<Response>>, sqlx::Error> {
    if !claims_in(db, at).await? {
        return Ok(Err(Box::new(not_a_member())));
    }
    if matches!(act, Act::Release) && at.proved.token.is_some() {
        return Ok(Err(Box::new(not_allowed(
            "A service device fails a task instead of releasing it.",
        ))));
    }
    let mut tx = db.begin().await?;
    let (task, held) = match locked(&mut tx, at, queue, task).await? {
        Ok(found) => found,
        Err(refused) => return Ok(Err(refused)),
    };
    // A claim ID that is none, or a generation PostgreSQL does not hold, names no claim.
    let (Ok(claim_id), Ok(generation)) = (
        named.claim_id.parse::<ClaimId>(),
        i64::try_from(named.generation),
    ) else {
        return Ok(Err(Box::new(claim_lost())));
    };
    let (claim_id, device) = (claim_id.to_string(), at.proved.device.to_string());
    let fenced = held.state == "claimed"
        && held.claim_id.as_ref() == Some(&claim_id)
        && held.generation == generation
        && held.claim_device.as_ref() == Some(&device)
        && held.lasts;
    if !fenced {
        let repeated = match act.ending() {
            Some(ending) => {
                let found: Option<(i32,)> = sqlx::query_as(
                    "SELECT 1 FROM claims
                     WHERE id = $1::uuid AND task_id = $2::uuid AND generation = $3
                       AND device_id = $4::uuid AND ended = $5",
                )
                .bind(&claim_id)
                .bind(&task)
                .bind(generation)
                .bind(&device)
                .bind(ending)
                .fetch_optional(&mut *tx)
                .await?;
                found.is_some()
            }
            None => false,
        };
        return Ok(if repeated {
            Ok(())
        } else {
            Err(Box::new(claim_lost()))
        });
    }
    let applied = Applied {
        task: &task,
        device: &device,
    };
    match act {
        Act::Extend(progress) => applied.extend(&mut tx, progress).await?,
        Act::Complete => applied.complete(&mut tx).await?,
        Act::Fail { retry } => {
            let again = retry
                && Mode::of(&held.mode) == Mode::AtLeastOnce
                && held.attempts < held.max_attempts;
            let backoff = again.then(|| backoff(held.attempts));
            applied.fail(&mut tx, backoff).await?;
        }
        Act::Release => applied.release(&mut tx, held.releases + 1).await?,
    }
    if let Some(ending) = act.ending() {
        sqlx::query("UPDATE claims SET ended = $2 WHERE id = $1::uuid")
            .bind(&claim_id)
            .bind(ending)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Ok(()))
}

/// The backoff after the `n`-th failed attempt, in seconds: `min(10 × 2^(n−1), 3600)`.
fn backoff(n: i32) -> i64 {
    let doublings = u32::try_from(n.saturating_sub(1)).unwrap_or(0).min(16);
    (FIRST_WAIT << doublings).min(LONGEST_BACKOFF)
}

/// The wait after the `n`-th release, n from 1 to 4, in seconds: `10 × 2^(n−1)`.
fn release_wait(n: i32) -> i64 {
    let doublings = u32::try_from(n.saturating_sub(1)).unwrap_or(0).min(16);
    FIRST_WAIT << doublings
}

/// An act the fencing check took, on `task` held by `device`.
struct Applied<'a> {
    task: &'a str,
    device: &'a str,
}

impl Applied<'_> {
    /// A new lease of the claim's length from now, with `progress`.
    async fn extend(&self, tx: &mut PgConnection, progress: Progress) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks t SET lease_until = now() + make_interval(secs => c.lease), progress = $2
             FROM claims c WHERE t.id = $1::uuid AND c.id = t.claim_id",
        )
        .bind(self.task)
        .bind(progress.as_str())
        .execute(tx)
        .await?;
        Ok(())
    }

    /// `done`.
    async fn complete(&self, tx: &mut PgConnection) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks SET state = 'done', progress = NULL, lease_until = NULL, since = now()
             WHERE id = $1::uuid",
        )
        .bind(self.task)
        .execute(tx)
        .await?;
        Ok(())
    }

    /// `queued` after `backoff` seconds if any, otherwise `dead` with `failed`.
    async fn fail(&self, tx: &mut PgConnection, backoff: Option<i64>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE tasks
             SET state = CASE WHEN $2::float8 IS NULL THEN 'dead' ELSE 'queued' END,
                 reason = CASE WHEN $2::float8 IS NULL THEN 'failed' END,
                 not_before = now() + make_interval(secs => $2::float8),
                 progress = NULL, lease_until = NULL, since = now()
             WHERE id = $1::uuid",
        )
        .bind(self.task)
        .bind(backoff.map(|seconds| seconds as f64))
        .execute(tx)
        .await?;
        Ok(())
    }

    /// The `n`-th release since enqueue or the last restart: `queued` at once, attempts minus one,
    /// kept from the releasing device for its wait; the 5th, `dead` with `released`.
    async fn release(&self, tx: &mut PgConnection, n: i32) -> Result<(), sqlx::Error> {
        let dead = n >= RELEASES;
        sqlx::query(
            "UPDATE tasks
             SET state = CASE WHEN $3 THEN 'dead' ELSE 'queued' END,
                 reason = CASE WHEN $3 THEN 'released' END,
                 attempts = attempts - 1, releases = $2, not_before = NULL, progress = NULL,
                 lease_until = NULL, since = now()
             WHERE id = $1::uuid",
        )
        .bind(self.task)
        .bind(n)
        .bind(dead)
        .execute(&mut *tx)
        .await?;
        if !dead {
            sqlx::query(
                "INSERT INTO task_waits (task_id, device_id, until)
                 VALUES ($1::uuid, $2::uuid, now() + make_interval(secs => $3::float8))
                 ON CONFLICT (task_id, device_id) DO UPDATE SET until = EXCLUDED.until",
            )
            .bind(self.task)
            .bind(self.device)
            .bind(release_wait(n) as f64)
            .execute(&mut *tx)
            .await?;
        }
        Ok(())
    }
}

/// `POST .../tasks/<task>/<change>`, of [`RESTART`] or [`CANCEL`]: a restart makes a `dead` task
/// `queued`, with attempts and releases 0; a cancel makes a `queued`, `claimed` or `dead` task
/// `cancelled`. `204`, or the refusal.
pub(crate) async fn change<const CHANGE: u8>(
    State(state): State<AppState>,
    at: InChannel,
    Path((_, _, queue, task)): TaskPath,
) -> Response {
    let change = if CHANGE == RESTART {
        Change::Restart
    } else {
        Change::Cancel
    };
    match changed(&state.db, &at, &queue, &task, change).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(refused)) => *refused,
        Err(err) => database_failed("restart or cancel a task", err),
    }
}

/// Makes `change` as [`change()`] says: only from a `client` or `browser` device in the space's
/// latest revision, of the uploader's account or of an admin.
async fn changed(
    db: &PgPool,
    at: &InChannel,
    queue: &str,
    task: &str,
    change: Change,
) -> Result<Result<(), Box<Response>>, sqlx::Error> {
    let refused = || {
        Err(Box::new(not_allowed(
            "Only a client or browser device of the uploader's account or of an admin restarts or \
             cancels a task.",
        )))
    };
    let (account, device) = (at.proved.account, at.proved.device);
    let Some(roster) = (latest(db, at.space).await?).filter(|r| listed(r.body(), account, device))
    else {
        return Ok(refused());
    };
    let kind: Option<(String,)> = sqlx::query_as("SELECT kind FROM devices WHERE id = $1::uuid")
        .bind(device.to_string())
        .fetch_optional(db)
        .await?;
    // A `service` device proves itself only on a token.
    if !kind.is_some_and(|(kind,)| kind == "client" || kind == "browser") {
        return Ok(refused());
    }
    let mut tx = db.begin().await?;
    let (task, found) = match locked(&mut tx, at, queue, task).await? {
        Ok(found) => found,
        Err(refused) => return Ok(Err(refused)),
    };
    if !(admin(&roster, account) || found.uploader == account.to_string()) {
        return Ok(refused());
    }
    match change {
        Change::Restart => {
            if found.state != "dead" {
                let refusal = conflict("not-dead", "Task not dead", "The task is not dead.");
                return Ok(Err(Box::new(refusal)));
            }
            sqlx::query(
                "UPDATE tasks
                 SET state = 'queued', attempts = 0, releases = 0, reason = NULL, not_before = NULL,
                     since = now()
                 WHERE id = $1::uuid",
            )
            .bind(&task)
            .execute(&mut *tx)
            .await?;
            sqlx::query("DELETE FROM task_waits WHERE task_id = $1::uuid")
                .bind(&task)
                .execute(&mut *tx)
                .await?;
        }
        Change::Cancel => {
            if found.state == "done" || found.state == "cancelled" {
                let detail = "The task is done or cancelled.";
                let refusal = conflict("not-cancellable", "Task not cancellable", detail);
                return Ok(Err(Box::new(refusal)));
            }
            sqlx::query(
                "UPDATE tasks
                 SET state = 'cancelled', progress = NULL, reason = NULL, lease_until = NULL,
                     not_before = NULL, since = now()
                 WHERE id = $1::uuid",
            )
            .bind(&task)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    Ok(Ok(()))
}

/// Whether `at`'s device passes the server's claim checks: a member device of the channel's current
/// epoch, of kind `client` or `service`, not revoked, and a `service` device on a token that allows
/// the channel's queues.
async fn claims_in(db: &PgPool, at: &InChannel) -> Result<bool, sqlx::Error> {
    let token = at.proved.token.as_ref();
    if token.is_some_and(|token| !token.enqueues_in(at.space, at.channel)) {
        return Ok(false);
    }
    let kind: Option<(String,)> = sqlx::query_as(
        "SELECT d.kind FROM devices d, channel_groups g JOIN channels c ON c.id = g.channel_id
         WHERE d.id = $1::uuid AND NOT d.revoked AND g.channel_id = $2::uuid
           AND c.space_id = $3::uuid AND d.id = ANY(g.members)",
    )
    .bind(at.proved.device.to_string())
    .bind(at.channel.to_string())
    .bind(at.space.to_string())
    .fetch_optional(db)
    .await?;
    Ok(match kind.as_ref().map(|(kind,)| kind.as_str()) {
        Some("client") => token.is_none(),
        Some("service") => token.is_some(),
        _ => false,
    })
}

/// The ID of the queue `queue` of `at`'s channel, if it has it.
async fn queue_of(
    db: &PgPool,
    at: &InChannel,
    queue: &str,
) -> Result<Option<QueueId>, sqlx::Error> {
    let Ok(queue) = queue.parse::<QueueId>() else {
        return Ok(None);
    };
    Ok((find(db, at.space, at.channel, queue).await?).map(|_| queue))
}

/// Applies lease ends and passed deadlines, and deletes finished tasks past `retention` if set, as
/// [`sweeps::every`] says.
pub(crate) async fn run(db: PgPool, retention: Option<Duration>) {
    let retention = retention.map(|retention| retention.as_secs_f64());
    sweeps::every("apply lease ends and deadlines", || {
        let db = db.clone();
        async move { sweep(&db, retention).await }
    })
    .await;
}

/// Lease ends: under `at-least-once` with attempts below `max_attempts`, `queued` after the backoff
/// from the lease's end; otherwise `dead` with `lease-expired`.
const LEASE_ENDS: &str = "
    WITH due AS (
        SELECT t.id, q.mode = 'at-least-once' AND t.attempts < q.max_attempts AS again
        FROM tasks t JOIN queues q ON q.id = t.queue_id
        WHERE t.state = 'claimed' AND t.lease_until <= now()
        ORDER BY t.lease_until LIMIT $1
        FOR UPDATE OF t SKIP LOCKED
    )
    UPDATE tasks t
    SET state = CASE WHEN due.again THEN 'queued' ELSE 'dead' END,
        reason = CASE WHEN due.again THEN NULL ELSE 'lease-expired' END,
        not_before = CASE WHEN due.again THEN t.lease_until + make_interval(
            secs => least(10 * power(2, t.attempts - 1), 3600)) END,
        progress = NULL, lease_until = NULL, since = now()
    FROM due WHERE t.id = due.id";

/// Passed deadlines of `queued` tasks: `dead` with `deadline-passed`.
const DEADLINES: &str = "
    WITH due AS (
        SELECT id FROM tasks WHERE state = 'queued' AND deadline <= now()
        ORDER BY deadline LIMIT $1
        FOR UPDATE SKIP LOCKED
    )
    UPDATE tasks t SET state = 'dead', reason = 'deadline-passed', not_before = NULL, since = now()
    FROM due WHERE t.id = due.id";

/// Finished tasks that got there longer ago than `$2` seconds: deleted.
const FINISHED: &str = "
    DELETE FROM tasks WHERE id IN (
        SELECT id FROM tasks
        WHERE state IN ('done', 'dead', 'cancelled') AND since <= now() - make_interval(secs => $2)
        LIMIT $1
        FOR UPDATE SKIP LOCKED
    )";

/// Applies what is due, a page at a time, as [`run`] says.
async fn sweep(db: &PgPool, retention: Option<f64>) -> Result<(), sqlx::Error> {
    let mut statements = vec![(LEASE_ENDS, None), (DEADLINES, None)];
    if let Some(retention) = retention {
        statements.push((FINISHED, Some(retention)));
    }
    for (statement, seconds) in statements {
        loop {
            let mut query = sqlx::query(statement).bind(PAGE);
            if let Some(seconds) = seconds {
                query = query.bind(seconds);
            }
            if query.execute(db).await?.rows_affected() == 0 {
                break;
            }
            // Leaves the database to other work between pages.
            tokio::time::sleep(PAUSE).await;
        }
    }
    Ok(())
}

fn not_a_member() -> Response {
    requests::not_a_member(
        "The device is no member device of the channel's current epoch, or its token has no \
         queues permission.",
    )
}

fn claim_lost() -> Response {
    conflict(
        "claim-lost",
        "Claim lost",
        "The claim is not the task's current one held by the device within its lease.",
    )
}

/// A `409` of `kind`, titled `title`, for the reason `detail`.
fn conflict(kind: &str, title: &str, detail: &str) -> Response {
    problem(StatusCode::CONFLICT, kind, title, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoffs_double_from_10_seconds_up_to_3600() {
        let backoffs: Vec<i64> = (1..=12).map(backoff).collect();
        let expected = [10, 20, 40, 80, 160, 320, 640, 1280, 2560, 3600, 3600, 3600];
        assert_eq!(backoffs, expected);
        assert_eq!(backoff(100), 3600);
    }

    #[test]
    fn release_waits_double_from_10_seconds() {
        let waits: Vec<i64> = (1..=4).map(release_wait).collect();
        assert_eq!(waits, [10, 20, 40, 80]);
    }
}
