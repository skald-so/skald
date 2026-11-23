//! Effective cloud plans. A space without an entitlement has the Free plan.

use serde::Serialize;
use skald_proto::SpaceId;
use sqlx::PgExecutor;

#[derive(Clone, Copy, Default, Serialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "space_plan", rename_all = "lowercase")]
pub(crate) enum SpacePlan {
    #[default]
    Free,
    Team,
}

pub(crate) struct PlanLimits {
    pub(crate) members: Option<u64>,
    pub(crate) devices_per_member: u64,
    pub(crate) service_agents: u64,
    pub(crate) monthly_messages: u64,
    pub(crate) message_size: u64,
    pub(crate) attachment_size: u64,
    /// Seconds.
    pub(crate) mailbox_retention: u64,
    /// Seconds.
    pub(crate) history_retention: u64,
}

const SECONDS_PER_DAY: u64 = 86_400;

const FREE: PlanLimits = PlanLimits {
    members: Some(3),
    devices_per_member: 10,
    service_agents: 3,
    monthly_messages: 1_000_000,
    message_size: 64 * 1024,
    attachment_size: 10 * 1024 * 1024,
    mailbox_retention: 7 * SECONDS_PER_DAY,
    history_retention: 30 * SECONDS_PER_DAY,
};

const TEAM: PlanLimits = PlanLimits {
    members: None,
    devices_per_member: 50,
    service_agents: 10,
    monthly_messages: 10_000_000,
    history_retention: 90 * SECONDS_PER_DAY,
    ..FREE
};

impl SpacePlan {
    pub(crate) fn limits(self) -> &'static PlanLimits {
        match self {
            Self::Free => &FREE,
            Self::Team => &TEAM,
        }
    }
}

/// Resolves the plan of an existing space. Plans off returns no plan and skips the lookup.
pub(crate) async fn resolve(
    db: impl PgExecutor<'_>,
    space: SpaceId,
    enabled: bool,
) -> Result<Option<SpacePlan>, sqlx::Error> {
    if !enabled {
        return Ok(None);
    }
    let plan = sqlx::query_scalar("SELECT plan FROM space_entitlements WHERE space_id = $1::uuid")
        .bind(space.to_string())
        .fetch_optional(db)
        .await?;
    Ok(Some(plan.unwrap_or_default()))
}
