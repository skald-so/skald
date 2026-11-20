//! Tokens of `service` devices for a space: an admin's session creates one, with the proof of one
//! of its account's `client` devices and the device's grant (`docs/protocol/README.md`, "Service
//! grant"); any admin of the space lists and revokes them, a token reads its own record, and the
//! space's members get the grants of those that receive ([`grants`]). The
//! value is given once and stored only as its SHA-256. A token ends at its expiry, at its
//! revocation, once its device is revoked and once the space's latest revision lists its device in
//! no service agent entry; its connections close then, on every instance. What a token reaches,
//! alone or with its device's proof: [`Token`].

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use skald_proto::{
    AccountId, Cbor as _, ChannelId, DeviceId, GRANT_LABEL, Grant, Role, Roster, ServiceAgent,
    SpaceId,
};
use sqlx::{PgConnection, PgPool};

use crate::devices::{base64, bytes};
use crate::refusal::{Answer, Refusal, account, answer, parse};
use crate::sessions::{close_ended, from_page, tell_ended};
use crate::sign_in::{bearer, digest, random_token};
use crate::spaces::{Found, Gotten, admin, find};
use crate::{AppState, database_failed, mailboxes, problem, requests};

/// The latest expiry a grant may have: the end of the year 9999, the last an RFC 3339 date-time
/// holds.
const LATEST_EXPIRY: u64 = 253_402_300_799;

/// The columns of a token's record, its date-times as RFC 3339 in UTC, from `service_tokens t`.
macro_rules! record_columns {
    () => {
        "t.id::text, t.device_id::text, t.permissions, t.channels::text[],
         to_char(t.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
         to_char(t.last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
         to_char(t.expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')"
    };
}

/// The condition that the token `t`, of the device `d`, has not ended by its expiry or its device's
/// revocation; a revocation of the token and a revision without its device's entry delete it.
macro_rules! live_token {
    () => {
        "t.expires_at > now() AND NOT d.revoked"
    };
}
pub(crate) use live_token;

/// A token's record as [`record_columns`] reads it.
type Row = (
    String,
    String,
    Vec<String>,
    Vec<String>,
    String,
    Option<String>,
    String,
);

/// `ServicePermission` in the spec.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
enum Permission {
    Send,
    Receive,
    Queues,
}

impl Permission {
    fn as_str(self) -> &'static str {
        match self {
            Permission::Send => "send",
            Permission::Receive => "receive",
            Permission::Queues => "queues",
        }
    }
}

/// `ServiceTokenRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenRequest {
    device: String,
    grant: String,
    permissions: Vec<Permission>,
}

/// `ServiceToken` in the spec.
#[derive(Serialize)]
struct Record {
    id: String,
    device: String,
    owner: String,
    agent: String,
    permissions: Vec<String>,
    channels: Vec<String>,
    created_at: String,
    last_used_at: Option<String>,
    expires_at: String,
}

/// `NewServiceToken` in the spec.
#[derive(Serialize)]
struct NewToken {
    #[serde(flatten)]
    record: Record,
    token: String,
}

/// `OwnServiceToken` in the spec.
#[derive(Serialize)]
struct OwnToken {
    #[serde(flatten)]
    record: Record,
    grant: String,
}

/// `ServiceTokens` in the spec.
#[derive(Serialize)]
struct Tokens {
    tokens: Vec<Record>,
}

/// The record of `row`, a token of the device that `roster` lists in a service agent entry; `None`
/// if it lists the device in none.
fn record(row: Row, roster: &Roster) -> Option<Record> {
    let (id, device, permissions, channels, created_at, last_used_at, expires_at) = row;
    let entry = entry(roster, &device)?;
    Some(Record {
        id,
        owner: entry.owner.to_string(),
        agent: entry.agent.to_string(),
        device,
        permissions,
        channels,
        created_at,
        last_used_at,
        expires_at,
    })
}

/// The service agent entry of `device`, a device ID, in `roster`.
fn entry<'r>(roster: &'r Roster, device: &str) -> Option<&'r ServiceAgent> {
    roster
        .body()
        .service_agents
        .iter()
        .find(|agent| agent.device.to_string() == device)
}

/// The latest revision of `space`, which an admin of it has.
async fn latest(db: impl sqlx::PgExecutor<'_>, space: SpaceId) -> Result<Roster, Refusal> {
    mailboxes::latest(db, space)
        .await
        .map_err(|err| Refusal::Database("read the latest revision", err))?
        .ok_or_else(|| Refusal::NotAllowed("the space has no revision yet".to_owned()))
}

/// Refuses `found` unless the session's account is an admin in its latest revision.
fn admin_only(found: &Found) -> Result<(), Refusal> {
    if found.admin {
        Ok(())
    } else {
        Err(Refusal::NotAllowed(
            "only an admin in the space's latest revision has its service tokens".to_owned(),
        ))
    }
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the Unix epoch")
        .as_secs()
}

/// `POST /spaces/<space>/service-tokens`: a token of a `service` device of the session's account,
/// listed in a service agent entry of the space's latest revision, with its grant, for a session
/// proved by one of the account's `client` devices while that account is an admin in the revision.
pub(crate) async fn create(
    State(state): State<AppState>,
    Path(space): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let proved = match requests::prove(&state, &headers).await {
        Ok(proved) => proved,
        Err(refused) => return refused,
    };
    let account = proved.account;
    drop(proved);
    answer(async {
        let request: TokenRequest = parse(&body)?;
        let mut permissions = request.permissions;
        permissions.sort();
        permissions.dedup();
        if permissions.is_empty() {
            return Err(Refusal::Invalid("permissions: none given".to_owned()));
        }
        let device: DeviceId = request
            .device
            .parse()
            .map_err(|_| Refusal::Invalid("device: not a device ID".to_owned()))?;
        let sent = bytes(&request.grant, "grant")?;
        let grant =
            Grant::from_cbor(&sent).map_err(|err| Refusal::Grant(format!("not a grant: {err}")))?;

        let db = &state.db;
        let found = find(db, &space, &account.to_string()).await?;
        if !found.gets {
            return Err(Refusal::NoSpace);
        }
        let failed = |err| Refusal::Database("store a service token", err);
        let mut tx = db.begin().await.map_err(failed)?;
        // Storing a revision locks the space's row (`spaces::insert`): until the token is stored, the
        // latest revision stays the one checked, so a revision that drops the entry ends it.
        sqlx::query("SELECT 1 FROM spaces WHERE id = $1::uuid FOR SHARE")
            .bind(found.id.to_string())
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
        let roster = latest(&mut *tx, found.id).await?;
        if !admin(&roster, account) {
            return Err(Refusal::NotAllowed(
                "only an admin in the space's latest revision creates its service tokens"
                    .to_owned(),
            ));
        }
        service_device(&mut tx, &roster, device, account).await?;
        check_grant(&mut tx, &roster, &grant, device).await?;

        let token = random_token();
        let permissions: Vec<&str> = permissions.iter().map(|p| p.as_str()).collect();
        let channels: Vec<String> = grant
            .body
            .channels
            .iter()
            .map(ToString::to_string)
            .collect();
        let expires = i64::try_from(grant.body.expires).expect("checked against the latest");
        let row: Row = sqlx::query_as(concat!(
            "INSERT INTO service_tokens AS t
                 (token_hash, space_id, device_id, permissions, channels, grant_cbor, expires_at)
             VALUES ($1, $2::uuid, $3::uuid, $4, $5::text[]::uuid[], $6, to_timestamp($7))
             RETURNING ",
            record_columns!()
        ))
        .bind(digest(&token))
        .bind(found.id.to_string())
        .bind(device.to_string())
        .bind(&permissions)
        .bind(&channels)
        .bind(&sent)
        .bind(expires)
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        let record = record(row, &roster).expect("the roster lists the device");
        Ok((
            StatusCode::CREATED,
            [(header::CACHE_CONTROL, "no-store")],
            Json(NewToken { record, token }),
        )
            .into_response())
    })
    .await
}

/// Refuses `device` unless it is a `service` device of `account` with a certificate, not revoked,
/// that `roster` lists in a service agent entry.
async fn service_device(
    conn: &mut PgConnection,
    roster: &Roster,
    device: DeviceId,
    account: AccountId,
) -> Result<(), Refusal> {
    let refused = || {
        Refusal::NotAllowed(
            "the device is not a service device of the session's account that the space's \
             latest revision lists"
                .to_owned(),
        )
    };
    if entry(roster, &device.to_string()).is_none() {
        return Err(refused());
    }
    let found: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid AND kind = 'service' AND cert IS NOT NULL
           AND NOT revoked",
    )
    .bind(device.to_string())
    .bind(account.to_string())
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    found.map(|_| ()).ok_or_else(refused)
}

/// Refuses `grant` unless it names `device`, its signer is an admin's device in `roster` whose
/// certificate its signature verifies with, it lists only channels of `roster`, and it has not
/// expired.
async fn check_grant(
    conn: &mut PgConnection,
    roster: &Roster,
    grant: &Grant,
    device: DeviceId,
) -> Result<(), Refusal> {
    let refused = |detail: &str| Err(Refusal::Grant(detail.to_owned()));
    let body = &grant.body;
    if body.service != device {
        return refused("the grant names another device");
    }
    let listed = |channel: &ChannelId| roster.body().channels.iter().any(|c| c.id == *channel);
    if !body.channels.iter().all(listed) {
        return refused("the grant lists a channel the space's latest revision does not");
    }
    if body.expires <= now() {
        return refused("the grant has expired");
    }
    if body.expires > LATEST_EXPIRY {
        return refused("the grant expires after the year 9999");
    }
    // An admin's device: a `client` or `browser` device with a certificate, not revoked, of an
    // admin in the revision whose `excluded` does not list it (`docs/protocol/membership.md`,
    // "Roster").
    let signer: Option<(String, Vec<u8>)> = sqlx::query_as(
        "SELECT account_id::text, signing FROM devices
         WHERE id = $1::uuid AND kind IN ('client', 'browser') AND cert IS NOT NULL
           AND NOT revoked",
    )
    .bind(body.signer.to_string())
    .fetch_optional(&mut *conn)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let admin = signer.and_then(|(account, signing)| {
        let account: AccountId = account.parse().ok()?;
        let signing: [u8; 32] = signing.try_into().ok()?;
        roster
            .body()
            .members
            .iter()
            .any(|member| {
                member.account == account
                    && member.role == Role::Admin
                    && !member
                        .excluded
                        .iter()
                        .flat_map(|excluded| excluded.iter())
                        .any(|excluded| *excluded == body.signer)
            })
            .then_some(signing)
    });
    let Some(signing) = admin else {
        return refused("the grant's signer is no admin's device in the space's latest revision");
    };
    if skald_crypto::verify(&signing, GRANT_LABEL, &[&body.to_cbor()], &grant.signature).is_err() {
        return refused("the grant's signature does not verify with its signer's certificate");
    }
    Ok(())
}

/// `GET /spaces/<space>/service-tokens`: the space's tokens that have not ended, for an admin's
/// session.
pub(crate) async fn list(State(state): State<AppState>, Gotten(found): Gotten) -> Answer {
    admin_only(&found)?;
    let roster = latest(&state.db, found.id).await?;
    let rows: Vec<Row> = sqlx::query_as(concat!(
        "SELECT ",
        record_columns!(),
        " FROM service_tokens t JOIN devices d ON d.id = t.device_id
         WHERE t.space_id = $1::uuid AND ",
        live_token!(),
        " ORDER BY t.created_at, t.id"
    ))
    .bind(found.id.to_string())
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the service tokens", err))?;
    let tokens = rows
        .into_iter()
        .filter_map(|row| record(row, &roster))
        .collect();
    Ok(Json(Tokens { tokens }).into_response())
}

/// `ServiceGrants` in the spec.
#[derive(Serialize)]
struct Grants {
    grants: Vec<String>,
}

/// `GET /spaces/<space>/service-grants`: the grants of the space's tokens that have not ended and
/// have the receive permission, oldest first, for a session of an account that gets the space.
pub(crate) async fn grants(State(state): State<AppState>, Gotten(found): Gotten) -> Answer {
    let rows: Vec<(Vec<u8>,)> = sqlx::query_as(concat!(
        "SELECT t.grant_cbor FROM service_tokens t JOIN devices d ON d.id = t.device_id
         WHERE t.space_id = $1::uuid AND 'receive' = ANY(t.permissions) AND ",
        live_token!(),
        " ORDER BY t.created_at, t.id",
    ))
    .bind(found.id.to_string())
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the service grants", err))?;
    let grants = rows.iter().map(|(grant,)| base64(grant)).collect();
    Ok(Json(Grants { grants }).into_response())
}

/// `DELETE /spaces/<space>/service-tokens/<token>`: ends that token of the space, for an admin's
/// session; a web session's only from a page of this server.
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Path((space, token)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        // Without an `Authorization` header, the session is the cookie's web session.
        if bearer(&headers).is_none() {
            from_page(&state, &headers)?;
        }
        let found = find(&state.db, &space, &account).await?;
        if !found.gets {
            return Err(Refusal::NoSpace);
        }
        admin_only(&found)?;
        // As text: an ID that is no UUID is no token. One that has ended stays as it is.
        let failed = |err| Refusal::Database("revoke a service token", err);
        let mut tx = state.db.begin().await.map_err(failed)?;
        let deleted: Vec<(Vec<u8>,)> = sqlx::query_as(concat!(
            "DELETE FROM service_tokens t USING devices d
             WHERE t.space_id = $1::uuid AND t.id::text = lower($2) AND d.id = t.device_id
                 AND ",
            live_token!(),
            " RETURNING t.token_hash",
        ))
        .bind(found.id.to_string())
        .bind(&token)
        .fetch_all(&mut *tx)
        .await
        .map_err(failed)?;
        if deleted.is_empty() {
            return Err(Refusal::NoServiceToken);
        }
        // Its connections close on every instance, as a session's at its end.
        tell_ended(&state, &mut tx, &deleted)
            .await
            .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        close_ended(&state, &deleted);
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

/// `GET /service-token`: the record and grant of the request's token, if it has not ended; records
/// its use, to the minute.
pub(crate) async fn own(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(Some(token)) = bearer(&headers) else {
        return no_token();
    };
    match own_record(&state.db, &digest(token)).await {
        Ok(Some(own)) => Json(own).into_response(),
        Ok(None) => no_token(),
        Err(err) => database_failed("read a service token", err),
    }
}

/// The record and grant of the token whose value has the SHA-256 `hash`, if it has not ended, with
/// its use recorded.
async fn own_record(db: &PgPool, hash: &[u8]) -> Result<Option<OwnToken>, sqlx::Error> {
    let found: Option<(String, Vec<u8>)> = sqlx::query_as(concat!(
        "SELECT t.space_id::text, t.grant_cbor FROM service_tokens t
         JOIN devices d ON d.id = t.device_id
         WHERE t.token_hash = $1 AND ",
        live_token!(),
    ))
    .bind(hash)
    .fetch_optional(db)
    .await?;
    let Some((space, grant)) = found else {
        return Ok(None);
    };
    let space: SpaceId = space.parse().expect("a stored space ID");
    let Some(roster) = mailboxes::latest(db, space).await? else {
        return Ok(None);
    };
    // The record after this use is recorded, to the minute.
    // A revocation meanwhile leaves no row.
    let row: Option<Row> = sqlx::query_as(concat!(
        "WITH used AS (
             UPDATE service_tokens SET last_used_at = now()
             WHERE token_hash = $1
                 AND (last_used_at IS NULL OR last_used_at <= now() - interval '1 minute')
             RETURNING id, last_used_at
         )
         SELECT ",
        record_columns!(),
        " FROM (SELECT s.id, s.device_id, s.permissions, s.channels, s.created_at, s.expires_at,
                 COALESCE(u.last_used_at, s.last_used_at) AS last_used_at
             FROM service_tokens s LEFT JOIN used u ON u.id = s.id WHERE s.token_hash = $1) t"
    ))
    .bind(hash)
    .fetch_optional(db)
    .await?;
    let Some(record) = row.and_then(|row| record(row, &roster)) else {
        return Ok(None);
    };
    Ok(Some(OwnToken {
        record,
        grant: base64(&grant),
    }))
}

/// A token that has not ended, as a request that carries it finds it: what it reaches.
#[derive(Clone)]
pub(crate) struct Token {
    pub(crate) space: SpaceId,
    /// Its `service` device, and that device's account.
    pub(crate) device: DeviceId,
    pub(crate) account: AccountId,
    send: bool,
    receive: bool,
    channels: Vec<ChannelId>,
    /// Seconds since the Unix epoch.
    pub(crate) expires: u64,
}

impl Token {
    /// Whether it lists `channel` of `space`.
    pub(crate) fn lists(&self, space: SpaceId, channel: ChannelId) -> bool {
        space == self.space && self.channels.contains(&channel)
    }

    /// Whether its device sends in `channel` of `space` with it.
    pub(crate) fn sends_in(&self, space: SpaceId, channel: ChannelId) -> bool {
        self.send && self.lists(space, channel)
    }

    /// Whether its device reads and acks its mailbox with it.
    pub(crate) fn receives(&self) -> bool {
        self.receive
    }
}

/// The spaces of `spaces` whose notices a connection of `token`, if it has one, gets: its token's
/// space only.
pub(crate) fn seen(token: Option<&Token>, mut spaces: Vec<SpaceId>) -> Vec<SpaceId> {
    if let Some(token) = token {
        spaces.retain(|space| *space == token.space);
    }
    spaces
}

/// The token whose value has the SHA-256 `hash`, if it has not ended, with its use recorded, to
/// the minute.
pub(crate) async fn live(db: &PgPool, hash: &[u8]) -> Result<Option<Token>, sqlx::Error> {
    type Found = (String, String, String, Vec<String>, Vec<String>, i64);
    let found: Option<Found> = sqlx::query_as(concat!(
        "WITH t AS (
             SELECT t.token_hash, t.space_id, t.device_id, d.account_id, t.permissions,
                 t.channels, t.expires_at, t.last_used_at
             FROM service_tokens t JOIN devices d ON d.id = t.device_id
             WHERE t.token_hash = $1 AND ",
        live_token!(),
        "), used AS (
             UPDATE service_tokens u SET last_used_at = now() FROM t
             WHERE u.token_hash = t.token_hash
                 AND (t.last_used_at IS NULL OR t.last_used_at <= now() - interval '1 minute')
         )
         SELECT space_id::text, device_id::text, account_id::text, permissions,
             channels::text[], floor(extract(epoch FROM expires_at))::bigint
         FROM t",
    ))
    .bind(hash)
    .fetch_optional(db)
    .await?;
    Ok(found.map(
        |(space, device, account, permissions, channels, expires)| Token {
            space: space.parse().expect("a stored space ID"),
            device: device.parse().expect("a stored device ID"),
            account: account.parse().expect("a stored account ID"),
            send: permissions.iter().any(|p| p == Permission::Send.as_str()),
            receive: permissions
                .iter()
                .any(|p| p == Permission::Receive.as_str()),
            channels: channels
                .iter()
                .map(|channel| channel.parse().expect("a stored channel ID"))
                .collect(),
            expires: u64::try_from(expires).expect("an expiry after 1970"),
        },
    ))
}

/// The answer to a request without a token that has not ended.
fn no_token() -> Response {
    problem(
        StatusCode::UNAUTHORIZED,
        "not-signed-in",
        "Not signed in",
        "The request has no service token, or an unknown or ended one.",
    )
}
