//! Space plans are opt-in. Usage resolves the plan per space, and only its members can read it.

use serde_json::{Value, json};
use skald_proto::SpaceId;

use crate::api::{assert_refused, space_of};
use crate::common::TestDb;
use crate::common::device_flow::{json as body_of, text};
use crate::common::devices::assert_taken;
use crate::sessions::Api;
use crate::usage::{Web, space_usage_response};

/// The last migration of the server before space plans.
const BEFORE_PLANS: i64 = 20270209000000;
const OPT_IN: [(&str, &str); 1] = [("SKALD_SPACE_PLANS", "true")];
pub(super) const CLOUD: &[&str] = &["--space-plans"];
const LEGACY_SETTINGS: &[&str] = &[
    "--monthly-messages-limit",
    "42",
    "--message-size-limit",
    "2048",
    "--mailbox-retention",
    "86400",
    "--history-retention",
    "172800",
    "--service-agents-limit",
    "8",
];

const DAY: u64 = 86_400;

#[track_caller]
fn usage(api: &Api, space: SpaceId, cookie: &str) -> Value {
    let response = space_usage_response(api, space, &[("cookie", cookie)]);
    assert_eq!(response.status, 200, "{}", text(&response));
    body_of(&response)
}

#[track_caller]
fn assert_plan(body: &Value, plan: &str, limits: [(&str, Option<u64>); 5]) {
    assert_eq!(body["plan"], plan, "effective plan: {body}");
    for (field, limit) in limits.into_iter().chain([
        ("message_size", Some(64 * 1024)),
        ("attachment_size", Some(10 * 1024 * 1024)),
        ("mailbox_retention", Some(7 * DAY)),
    ]) {
        assert!(body[field].is_object(), "{plan}: missing {field}: {body}");
        assert_eq!(
            body[field].get("limit"),
            limit.map(|limit| json!(limit)).as_ref(),
            "{plan}: {field}: {body}"
        );
    }
    assert_eq!(body["monthly_messages"]["count"], 0, "{body}");
    assert_eq!(body["service_agents"]["count"], 0, "{body}");
}

#[track_caller]
fn assert_free(body: &Value) {
    assert_plan(
        body,
        "free",
        [
            ("members", Some(3)),
            ("devices_per_member", Some(10)),
            ("service_agents", Some(3)),
            ("monthly_messages", Some(1_000_000)),
            ("history_retention", Some(30 * DAY)),
        ],
    );
}

/// The only fixture write for a paid plan. Usage itself is read through the public API.
pub(super) fn entitle_team(db: &TestDb, space: SpaceId) {
    db.query(async |conn| {
        sqlx::query("INSERT INTO space_entitlements (space_id, plan) VALUES ($1::uuid, 'team')")
            .bind(space.to_string())
            .execute(conn)
            .await
    })
    .expect("seed a Team entitlement in the upgraded database");
}

/// Removes a fixture entitlement so the space goes back to the default plan.
pub(super) fn remove_entitlement(db: &TestDb, space: SpaceId) {
    db.query(async |conn| {
        sqlx::query("DELETE FROM space_entitlements WHERE space_id = $1::uuid")
            .bind(space.to_string())
            .execute(conn)
            .await
    })
    .expect("remove the fixture Team entitlement");
}

#[test]
fn existing_and_new_spaces_default_to_free_with_plans_on() {
    let web = Web::with(3_226_900_001, &[]);
    let old = web.fixture.db().snapshot_at(BEFORE_PLANS);
    let team = &web.fixture.team;
    let cloud = team
        .flow
        .stand
        .instance_at(&old.url(), LEGACY_SETTINGS, &OPT_IN);
    let api = web.api();
    let api = api.on(&cloud);

    assert_free(&usage(&api, team.space, &web.carol));

    let created = team.alice.session.on(&cloud).create("globex");
    assert_taken(&created);
    assert_free(&usage(&api, space_of(&created), &web.alice));
}

#[test]
fn only_members_see_team_usage_and_plans_off_keep_legacy_limits() {
    let web = Web::with(3_226_910_001, &[]);
    let team = &web.fixture.team;
    let free = team.alice.create_space("globex");
    let old = web.fixture.db().snapshot_at(BEFORE_PLANS);
    let stand = &team.flow.stand;
    drop(stand.instance_at(&old.url(), &[], &[]));
    entitle_team(&old, team.space);
    let cloud = stand.instance_at(&old.url(), LEGACY_SETTINGS, &OPT_IN);
    let api = web.api();
    let cloud_api = api.on(&cloud);

    for member in [&web.alice, &web.carol] {
        assert_plan(
            &usage(&cloud_api, team.space, member),
            "team",
            [
                ("members", None),
                ("devices_per_member", Some(50)),
                ("service_agents", Some(10)),
                ("monthly_messages", Some(10_000_000)),
                ("history_retention", Some(90 * DAY)),
            ],
        );
    }
    assert_free(&usage(&cloud_api, free, &web.alice));

    let eve = [("cookie", web.eve.as_str())];
    let refused = space_usage_response(&cloud_api, team.space, &eve);
    let unknown = space_usage_response(&cloud_api, SpaceId::generate(), &eve);
    assert_refused(&refused, "an outsider requesting Team usage");
    assert_eq!(refused.status, unknown.status);
    assert_eq!(body_of(&refused), body_of(&unknown));
    let anonymous = space_usage_response(&cloud_api, team.space, &[]);
    assert_eq!(anonymous.status, 401, "{}", text(&anonymous));

    drop(cloud);
    let legacy = stand.instance_at(
        &old.url(),
        LEGACY_SETTINGS,
        &[("SKALD_SPACE_PLANS", "false")],
    );
    assert_eq!(
        usage(&api.on(&legacy), team.space, &web.carol),
        json!({
            "monthly_messages": { "count": 0, "limit": 42 },
            "message_size": { "limit": 2048 },
            "mailbox_retention": { "limit": DAY },
            "history_retention": { "limit": 2 * DAY },
            "service_agents": { "count": 0, "limit": 8 },
        }),
        "plans explicitly off"
    );

    drop(legacy);
    let self_hosted = stand.instance_at(&old.url(), &[], &[]);
    assert_eq!(
        usage(&api.on(&self_hosted), team.space, &web.carol),
        json!({
            "monthly_messages": { "count": 0 },
            "message_size": {},
            "mailbox_retention": {},
            "history_retention": {},
            "service_agents": { "count": 0 },
        }),
        "self-hosted without settings"
    );
}
