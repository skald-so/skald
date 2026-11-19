//! What the web app's Usage section shows (`docs/dev/architecture.md`, "Cloud limits"): an
//! account's owned spaces and devices, and a space's messages this month and service agents, each
//! counted as its limit counts it, with the limit while the server has its setting.

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;

use crate::AppState;
use crate::mailboxes;
use crate::refusal::{Answer, Refusal, SignedIn};
use crate::spaces::{Gotten, Space};

/// `UsageCount` in the spec.
#[derive(Serialize)]
struct UsageCount {
    count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
}

/// `UsageLimit` in the spec.
#[derive(Serialize)]
struct UsageLimit {
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
}

/// `OwnedSpaces` in the spec.
#[derive(Serialize)]
struct OwnedSpaces {
    count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    spaces: Vec<Space>,
}

/// `AccountUsage` in the spec.
#[derive(Serialize)]
struct AccountUsage {
    owned_spaces: OwnedSpaces,
    devices: UsageCount,
}

/// `SpaceUsage` in the spec.
#[derive(Serialize)]
struct SpaceUsage {
    monthly_messages: UsageCount,
    message_size: UsageLimit,
    mailbox_retention: UsageLimit,
    history_retention: UsageLimit,
    service_agents: UsageCount,
}

/// A count the database gives, never negative.
fn counted(count: i64) -> u64 {
    u64::try_from(count).expect("a count is not negative")
}

/// `GET /account/usage`: the spaces the session's account owns, its personal space counted also
/// before it is created, and its devices with a certificate, `service` and revoked ones aside.
pub(crate) async fn account(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let failed = |err| Refusal::Database("read the account's usage", err);
    let created: Vec<(String, String, bool)> = sqlx::query_as(
        "SELECT id::text, name, personal FROM spaces WHERE creator_id = $1::uuid
         ORDER BY created_at, id",
    )
    .bind(&account)
    .fetch_all(&state.db)
    .await
    .map_err(failed)?;
    let (devices,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM devices
         WHERE account_id = $1::uuid AND cert IS NOT NULL AND kind <> 'service' AND NOT revoked",
    )
    .bind(&account)
    .fetch_one(&state.db)
    .await
    .map_err(failed)?;
    // As `POST /spaces` counts them: the personal space counts whether or not it is created yet.
    let team = created.iter().filter(|(_, _, personal)| !personal).count();
    let owned_spaces = OwnedSpaces {
        count: team as u64 + 1,
        limit: state.owned_spaces_limit.map(u64::from),
        spaces: created
            .into_iter()
            .map(|(space, name, _)| Space { space, name })
            .collect(),
    };
    let devices = UsageCount {
        count: counted(devices),
        limit: state.devices_limit.map(u64::from),
    };
    Ok(Json(AccountUsage {
        owned_spaces,
        devices,
    })
    .into_response())
}

/// `GET /spaces/<space>/usage`: the space's messages this calendar month in UTC, as the monthly
/// limit counts them, the limits on its messages and history, and the service agent entries of its
/// latest revision.
pub(crate) async fn space(State(state): State<AppState>, Gotten(found): Gotten) -> Answer {
    let messages: Option<(i64,)> = sqlx::query_as(
        "SELECT messages FROM message_counts
         WHERE space_id = $1::uuid AND month = date_trunc('month', now() AT TIME ZONE 'UTC')::date",
    )
    .bind(found.id.to_string())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the space's messages this month", err))?;
    let latest = mailboxes::latest(&state.db, found.id)
        .await
        .map_err(|err| Refusal::Database("read the latest revision", err))?;
    let agents = latest.map_or(0, |roster| roster.body().service_agents.len());
    Ok(Json(SpaceUsage {
        monthly_messages: UsageCount {
            count: messages.map_or(0, |(count,)| counted(count)),
            limit: state.limits.monthly_messages,
        },
        message_size: UsageLimit {
            limit: state.limits.message_size,
        },
        mailbox_retention: UsageLimit {
            limit: state.mailbox_retention.map(|retention| retention.as_secs()),
        },
        history_retention: UsageLimit {
            limit: state.history_retention.map(|retention| retention.as_secs()),
        },
        service_agents: UsageCount {
            count: agents as u64,
            limit: state.service_agents_limit.map(u64::from),
        },
    })
    .into_response())
}
