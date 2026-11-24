//! A current member can submit a structured report about another member of the same space.

use std::num::NonZeroU32;

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Request, State, rejection::PathRejection};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use skald_proto::{AccountId, MessageId, Name, Roster, SpaceId};
use sqlx::{PgConnection, PgPool};

use crate::refusal::{Answer, Refusal, SignedIn, parse};
use crate::sign_in::{digest, random_token};
use crate::sweeps::{self, PAGE, PAUSE};
use crate::{AppState, mailboxes, sessions};

/// A report contains only a handle, a reason and an idempotency key.
const BODY_LIMIT: usize = 1024;

/// The server's opt-in settings for member reports.
#[derive(Debug, Clone)]
pub struct AbuseReportingSettings {
    reasons: Vec<String>,
    limit: NonZeroU32,
    capacity: NonZeroU32,
    retention_seconds: NonZeroU32,
}

impl AbuseReportingSettings {
    /// Takes comma-separated category names, the per-account daily limit, the server's total
    /// report capacity and the retention in seconds for new reports.
    pub fn new(
        reasons: &str,
        limit: NonZeroU32,
        capacity: NonZeroU32,
        retention_seconds: NonZeroU32,
    ) -> Result<Self, &'static str> {
        let mut categories = Vec::new();
        for reason in reasons.split(',').map(str::trim) {
            if reason.parse::<Name>().is_err() || categories.iter().any(|known| known == reason) {
                return Err("abuse report reasons must be distinct category names");
            }
            categories.push(reason.to_owned());
        }
        Ok(Self {
            reasons: categories,
            limit,
            capacity,
            retention_seconds,
        })
    }
}

#[derive(Serialize)]
struct Capability<'a> {
    reasons: &'a [String],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    target: String,
    reason: String,
    idempotency_key: String,
}

#[derive(Serialize)]
struct Receipt {
    receipt: String,
}

fn failed(err: sqlx::Error) -> Refusal {
    Refusal::Database("process abuse reports", err)
}

/// Prevents caching the member-only capability, receipts and refusals.
pub(crate) async fn no_store(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The latest roster lists the reporter as a member. Before revision 1, there are no members.
async fn roster(
    db: impl sqlx::PgExecutor<'_>,
    space: SpaceId,
    reporter: &str,
) -> Result<Roster, Refusal> {
    let reporter: AccountId = reporter.parse().expect("a stored account ID");
    (mailboxes::latest(db, space).await.map_err(failed)?)
        .filter(|roster| roster.body().members.iter().any(|m| m.account == reporter))
        .ok_or(Refusal::NoSpace)
}

fn space_id(path: Result<Path<String>, PathRejection>) -> Result<SpaceId, Refusal> {
    path.map_err(|_| Refusal::NoSpace)?
        .0
        .parse()
        .map_err(|_| Refusal::NoSpace)
}

/// `GET /spaces/<space>/abuse-reporting`: the available reason categories for a current member.
pub(crate) async fn capability(
    State(state): State<AppState>,
    SignedIn(reporter): SignedIn,
    path: Result<Path<String>, PathRejection>,
) -> Answer {
    let settings = state
        .abuse_reporting
        .as_deref()
        .ok_or(Refusal::AbuseReportingOff)?;
    roster(&state.db, space_id(path)?, &reporter).await?;
    Ok(Json(Capability {
        reasons: &settings.reasons,
    })
    .into_response())
}

/// `POST /spaces/<space>/abuse-reports`: stores one report or returns the receipt of a retry.
pub(crate) async fn submit(
    State(state): State<AppState>,
    SignedIn(reporter): SignedIn,
    path: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Body,
) -> Answer {
    let settings = state
        .abuse_reporting
        .as_deref()
        .ok_or(Refusal::AbuseReportingOff)?;
    if !headers.contains_key(header::AUTHORIZATION) {
        sessions::from_page(&state, &headers)?;
    }
    let space = space_id(path)?;
    let body = to_bytes(body, BODY_LIMIT)
        .await
        .map_err(|_| Refusal::AbuseReportTooLarge)?;
    let Submission {
        target,
        reason,
        idempotency_key,
    } = parse(&body).map_err(|_| invalid())?;
    let target: Name = target.parse().map_err(|_| invalid())?;
    let _: MessageId = idempotency_key.parse().map_err(|_| invalid())?;
    let hash = digest(&idempotency_key);

    let mut tx = state.db.begin().await.map_err(failed)?;
    let target = report_target(&mut tx, space, &reporter, &target).await?;
    if !settings.reasons.contains(&reason) {
        return Err(invalid());
    }
    let space = space.to_string();

    // All submitters serialize the retry and limit checks with the insert, including across
    // spaces and serving instances. Sweeps can only reduce the count.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('abuse reports', 0))")
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    let previous: Option<(String, String, String)> = sqlx::query_as(
        "SELECT receipt, target_id::text, reason FROM abuse_reports
         WHERE space_id = $1::uuid AND reporter_id = $2::uuid AND idempotency_hash = $3
         FOR KEY SHARE",
    )
    .bind(&space)
    .bind(&reporter)
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    if let Some((receipt, previous_target, previous_reason)) = previous {
        if previous_target != target || previous_reason != reason {
            return Err(Refusal::AbuseReportConflict);
        }
        tx.commit().await.map_err(failed)?;
        return Ok(Json(Receipt { receipt }).into_response());
    }
    check_limits(&mut tx, &reporter, settings).await?;

    let receipt = random_token();
    sqlx::query(
        "INSERT INTO abuse_reports
             (receipt, space_id, reporter_id, target_id, reason, idempotency_hash, retention_seconds)
         VALUES ($1, $2::uuid, $3::uuid, $4::uuid, $5, $6, $7)",
    )
    .bind(&receipt)
    .bind(&space)
    .bind(&reporter)
    .bind(target)
    .bind(reason)
    .bind(hash)
    .bind(i64::from(settings.retention_seconds.get()))
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    Ok((StatusCode::CREATED, Json(Receipt { receipt })).into_response())
}

fn invalid() -> Refusal {
    Refusal::Invalid("Give a member handle, an available reason and an idempotency key.".to_owned())
}

/// Resolves the handle in the locked space's latest roster. Account locks keep both active until
/// the transaction ends, in the same order as roster revisions and account deletion.
async fn report_target(
    tx: &mut PgConnection,
    space: SpaceId,
    reporter: &str,
    target: &Name,
) -> Result<String, Refusal> {
    sqlx::query("SELECT 1 FROM spaces WHERE id = $1::uuid FOR SHARE")
        .bind(space.to_string())
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    let roster = roster(&mut *tx, space, reporter).await?;
    let target = roster
        .body()
        .members
        .iter()
        .find(|member| &member.handle == target)
        .map(|member| member.account.to_string())
        .filter(|target| target != reporter)
        .ok_or(Refusal::NoAbuseReportTarget)?;
    let active: Vec<String> = sqlx::query_scalar(
        "SELECT id::text FROM accounts
         WHERE id IN ($1::uuid, $2::uuid) AND deleted_at IS NULL
         ORDER BY id FOR KEY SHARE",
    )
    .bind(reporter)
    .bind(&target)
    .fetch_all(&mut *tx)
    .await
    .map_err(failed)?;
    if !active.iter().any(|id| id == reporter) {
        return Err(Refusal::NotSignedIn);
    }
    if active.len() != 2 {
        return Err(Refusal::NoAbuseReportTarget);
    }
    Ok(target)
}

async fn check_limits(
    tx: &mut PgConnection,
    reporter: &str,
    settings: &AbuseReportingSettings,
) -> Result<(), Refusal> {
    let full: bool = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM abuse_reports
                 WHERE reporter_id = $1::uuid
                   AND submitted_at > statement_timestamp() - interval '24 hours') >= $2
             OR (SELECT count(*) FROM abuse_reports) >= $3",
    )
    .bind(reporter)
    .bind(i64::from(settings.limit.get()))
    .bind(i64::from(settings.capacity.get()))
    .fetch_one(tx)
    .await
    .map_err(failed)?;
    if full {
        return Err(Refusal::AbuseReportLimit);
    }
    Ok(())
}

/// Deletes expired reports even if intake has since been turned off.
pub(crate) async fn run(db: PgPool) {
    sweeps::every("delete expired abuse reports", || async {
        loop {
            let deleted = sqlx::query(
                "DELETE FROM abuse_reports WHERE receipt IN (
                     SELECT receipt FROM abuse_reports
                     WHERE expires_at <= (now() AT TIME ZONE 'UTC')
                     ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED
                 )",
            )
            .bind(PAGE)
            .execute(&db)
            .await?;
            if deleted.rows_affected() < PAGE as u64 {
                return Ok(());
            }
            tokio::time::sleep(PAUSE).await;
        }
    })
    .await;
}
