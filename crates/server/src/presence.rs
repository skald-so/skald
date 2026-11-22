//! Presence on the server: `docs/protocol/agents.md`, "Presence", "Uploads". Each request proves
//! its device as `/mailbox/send` does, so a session alone uploads and reads nothing. The server
//! takes a device's value in a channel only from a member device of the channel's current epoch,
//! naming itself and that epoch, and keeps the latest one per channel and device until a Commit it
//! accepts removes the device from the group, or the device is revoked. It gives a channel's
//! values, as the bytes it got, only to devices that may send in the channel
//! (`docs/protocol/delivery.md`, "Send").

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use skald_proto::{Cbor as _, ChannelId, DeviceId, Presence, SpaceId};
use sqlx::{PgConnection, PgPool};

use crate::devices::base64;
use crate::mailboxes::receiving;
use crate::requests::{self, InChannel, invalid};
use crate::{AppState, database_failed, problem};

/// The longest `ct` the server takes: 16 blocks of 4096 bytes and the tag.
const LONGEST_CT: usize = 16 * 4096 + 16;
/// The longest body of an upload: a value with the longest `ct` and room for its other fields.
const BODY_LIMIT: usize = 2 * LONGEST_CT;

/// The answer to a request for a channel's presence that the server refuses, or its failure.
type Refused = Box<Response>;

/// `PUT /spaces/<space>/channels/<channel>/presence`: keeps the body, a `presence` value, as the
/// proving device's latest in the channel, in place of the one before.
pub(crate) async fn upload(State(state): State<AppState>, at: InChannel, body: Body) -> Response {
    let Ok(body) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return too_large();
    };
    let InChannel {
        proved,
        space,
        channel,
    } = at;
    let Ok(value) = Presence::from_cbor(&body) else {
        return invalid("The body is not a presence value.");
    };
    if value.channel != channel {
        return invalid("The presence value is of another channel.");
    }
    if value.ct.len() > LONGEST_CT {
        return too_large();
    }
    // A `service` device uploads with its token only in the token's channels.
    let listed = (proved.token.as_ref()).is_none_or(|token| token.lists(space, channel));
    if value.device != proved.device || !listed {
        return not_a_member();
    }
    match keep(&state.db, space, &value, &body).await {
        Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
        Ok(Err(refused)) => *refused,
        Err(err) => database_failed("keep a presence value", err),
    }
}

/// Keeps `value`, of `bytes`, in `db` if its device is a member of its channel's current epoch in
/// `space`, which it names; otherwise the refusal.
async fn keep(
    db: &PgPool,
    space: SpaceId,
    value: &Presence,
    bytes: &[u8],
) -> Result<Result<(), Refused>, sqlx::Error> {
    let mut tx = db.begin().await?;
    // The group's row lock makes a Commit wait for the upload, or the upload see it; the device's,
    // taken by `receiving`, a revocation.
    let Some(current) = current(&mut tx, space, value.channel, value.device).await? else {
        return Ok(Err(Box::new(not_a_member())));
    };
    if current != value.epoch {
        return Ok(Err(Box::new(problem(
            StatusCode::CONFLICT,
            "epoch-mismatch",
            "Another epoch",
            "The presence value is not of the channel's current epoch.",
        ))));
    }
    sqlx::query(
        "INSERT INTO presence_values (channel_id, device_id, value) VALUES ($1::uuid, $2::uuid, $3)
         ON CONFLICT (channel_id, device_id) DO UPDATE SET value = excluded.value",
    )
    .bind(value.channel.to_string())
    .bind(value.device.to_string())
    .bind(bytes)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Ok(()))
}

/// The current epoch of `channel` of `space` if `device` is a member of it and not revoked; the
/// group and the device locked on `tx` until it commits.
async fn current(
    tx: &mut PgConnection,
    space: SpaceId,
    channel: ChannelId,
    device: DeviceId,
) -> Result<Option<u64>, sqlx::Error> {
    let found: Option<(i64,)> = sqlx::query_as(
        "SELECT g.epoch FROM channel_groups g JOIN channels c ON c.id = g.channel_id
         WHERE g.channel_id = $1::uuid AND c.space_id = $2::uuid AND g.view IS NOT NULL
           AND $3::uuid = ANY(g.members)
         FOR SHARE OF g",
    )
    .bind(channel.to_string())
    .bind(space.to_string())
    .bind(device.to_string())
    .fetch_optional(&mut *tx)
    .await?;
    let Some((current,)) = found else {
        return Ok(None);
    };
    if receiving(tx, &[device]).await?.is_empty() {
        return Ok(None);
    }
    Ok(Some(u64::try_from(current).expect("a stored epoch from 1")))
}

/// `GET /spaces/<space>/channels/<channel>/presence`: the channel's values, as the bytes the server
/// got, base64url without padding, for a device that may send in the channel.
pub(crate) async fn given(State(state): State<AppState>, at: InChannel) -> Response {
    match values(&state, &at).await {
        Ok(Some(values)) => Json(serde_json::json!({ "values": values })).into_response(),
        Ok(None) => not_a_member(),
        Err(err) => database_failed("read a channel's presence", err),
    }
}

/// The values of `at`'s channel, base64url without padding, if its device may send in it.
async fn values(state: &AppState, at: &InChannel) -> Result<Option<Vec<String>>, sqlx::Error> {
    if !sends(state, at).await? {
        return Ok(None);
    }
    let values: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT value FROM presence_values WHERE channel_id = $1::uuid")
            .bind(at.channel.to_string())
            .fetch_all(&state.db)
            .await?;
    Ok(Some(values.iter().map(|(bytes,)| base64(bytes)).collect()))
}

/// Whether `at`'s device may send in its channel ([`InChannel::sends`]) and is a member of the
/// channel's current epoch.
async fn sends(state: &AppState, at: &InChannel) -> Result<bool, sqlx::Error> {
    if !at.sends(&state.db).await? {
        return Ok(false);
    }
    let mut tx = state.db.begin().await?;
    let member = current(&mut tx, at.space, at.channel, at.proved.device).await?;
    tx.commit().await?;
    Ok(member.is_some())
}

/// Deletes, on `tx`, the values in `channel` of the devices not in `members`, the member devices of
/// its group's new epoch.
pub(crate) async fn keep_members(
    tx: &mut PgConnection,
    channel: ChannelId,
    members: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM presence_values
         WHERE channel_id = $1::uuid AND device_id <> ALL($2::text[]::uuid[])",
    )
    .bind(channel.to_string())
    .bind(members)
    .execute(tx)
    .await?;
    Ok(())
}

/// Deletes, on `tx`, every value of `devices`, newly revoked.
pub(crate) async fn revoked(tx: &mut PgConnection, devices: &[String]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM presence_values WHERE device_id = ANY($1::text[]::uuid[])")
        .bind(devices)
        .execute(tx)
        .await?;
    Ok(())
}

fn not_a_member() -> Response {
    requests::not_a_member(
        "The device is not a member of the channel's current epoch, or may not do this in it.",
    )
}

fn too_large() -> Response {
    problem(
        StatusCode::PAYLOAD_TOO_LARGE,
        "too-large",
        "Value too large",
        "The presence value's ct is over 65552 bytes.",
    )
}
