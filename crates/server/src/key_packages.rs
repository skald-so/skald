//! KeyPackages (`docs/protocol/membership.md`, "KeyPackages"): a session of a `client` device's
//! account uploads the device's KeyPackages and its last-resort one, which the server checks as a
//! device checks one before an Add, and reads how many it has; sessions of accounts that share a
//! space with the device take one each, each other KeyPackage once, then the last-resort one.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use skald_proto::{Cbor as _, Cert, DeviceId};
use sqlx::PgPool;

use crate::AppState;
use crate::devices::{base64, bytes};
use crate::refusal::{Answer, Caller, Refusal, SignedIn, parse};

/// The most unused KeyPackages, the last-resort one aside, a device may hold: five times the 20 it
/// keeps.
pub(crate) const HELD_LIMIT: i64 = 100;

/// `KeyPackageUpload` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    #[serde(default)]
    key_packages: Vec<String>,
    last_resort: Option<String>,
}

/// `KeyPackageCount` in the spec.
#[derive(Serialize)]
struct Count {
    unused: i64,
    replace_last_resort: bool,
}

/// `KeyPackageRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    devices: Vec<String>,
}

/// `HandedOutKeyPackages` in the spec.
#[derive(Serialize)]
struct HandedOut {
    key_packages: Vec<Handed>,
}

/// `HandedOutKeyPackage` in the spec.
#[derive(Serialize)]
struct Handed {
    device: String,
    key_package: String,
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The device `device`, a path parameter, of the session's `account`: its ID, and its certificate
/// if it is a `client` device with one that is not revoked.
async fn own_device(
    db: &PgPool,
    device: &str,
    account: &str,
) -> Result<(String, Option<Vec<u8>>), Refusal> {
    device_of(db, device, account, "client").await
}

/// The device `device`, a path parameter, of `account`: its ID, and its certificate if it is a
/// device of `kind` with one that is not revoked.
async fn device_of(
    db: &PgPool,
    device: &str,
    account: &str,
    kind: &str,
) -> Result<(String, Option<Vec<u8>>), Refusal> {
    let id: DeviceId = device.parse().map_err(|_| Refusal::NoDevice)?;
    let id = id.to_string();
    let found: Option<(Option<Vec<u8>>,)> = sqlx::query_as(
        "SELECT CASE WHEN kind = $3 AND NOT revoked THEN cert END FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid",
    )
    .bind(&id)
    .bind(account)
    .bind(kind)
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let (cert,) = found.ok_or(Refusal::NoDevice)?;
    Ok((id, cert))
}

/// Deletes the expired KeyPackages of `device`.
async fn delete_expired(db: &PgPool, device: &str) -> Result<(), Refusal> {
    sqlx::query("DELETE FROM key_packages WHERE device_id = $1::uuid AND expires_at <= now()")
        .bind(device)
        .execute(db)
        .await
        .map_err(|err| Refusal::Database("delete expired KeyPackages", err))?;
    Ok(())
}

/// How many unused KeyPackages `device` has that expire in more than 7 days, and whether its
/// last-resort one is missing, was handed out or expires within 7 days.
async fn count(db: &PgPool, device: &str) -> Answer {
    delete_expired(db, device).await?;
    let (unused, kept): (i64, bool) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE NOT last_resort AND expires_at > now() + interval '7 days'),
             COALESCE(bool_or(last_resort AND NOT handed_out
                 AND expires_at > now() + interval '7 days'), false)
         FROM key_packages WHERE device_id = $1::uuid",
    )
    .bind(device)
    .fetch_one(db)
    .await
    .map_err(|err| Refusal::Database("count KeyPackages", err))?;
    Ok(Json(Count {
        unused,
        replace_last_resort: !kept,
    })
    .into_response())
}

/// `GET /devices/<device>/key-packages`: the count of a device of the session's account.
pub(crate) async fn status(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(device): Path<String>,
) -> Answer {
    let (device, _) = own_device(&state.db, &device, &account).await?;
    count(&state.db, &device).await
}

/// The KeyPackages of `texts`, each checked against `cert` at `now`, and as a last-resort one,
/// which carries the `last_resort` extension, if `last_resort`: its bytes and the end of its
/// lifetime, in seconds since the Unix epoch.
fn checked(
    texts: &[String],
    cert: &Cert,
    now: u64,
    last_resort: bool,
) -> Result<Vec<(Vec<u8>, i64)>, Refusal> {
    let field = if last_resort {
        "last_resort"
    } else {
        "key_packages"
    };
    texts
        .iter()
        .map(|text| {
            let sent = bytes(text, field)?;
            let checked = skald_crypto::check_key_package(&sent, cert, now)
                .map_err(|err| Refusal::KeyPackage(err.to_string()))?;
            if last_resort && !checked.last_resort {
                return Err(Refusal::KeyPackage(
                    "the last-resort one lacks the `last_resort` extension".to_owned(),
                ));
            }
            let expires = i64::try_from(checked.expires)
                .map_err(|_| Refusal::KeyPackage("its lifetime ends too late".to_owned()))?;
            Ok((sent, expires))
        })
        .collect()
}

/// `POST /devices/<device>/key-packages`: takes KeyPackages of a `client` device of the session's
/// account, or of a service token's `service` device, with a certificate, not revoked, and its
/// last-resort one, which replaces the one it had; one that fails the checks refuses them all.
/// Gives the device's count.
pub(crate) async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    Path(device): Path<String>,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let (device, cert) = match &caller {
        Caller::Session(account) => own_device(db, &device, account).await?,
        Caller::Token(token) => {
            if device.parse::<DeviceId>().ok() != Some(token.device) {
                return Err(Refusal::NoDevice);
            }
            device_of(db, &device, &caller.account(), "service").await?
        }
    };
    let upload: Upload = parse(&body)?;
    let cert = cert.ok_or_else(|| {
        Refusal::NotAllowed(
            "only a device with a certificate, not revoked, has KeyPackages".to_owned(),
        )
    })?;
    let cert = Cert::from_cbor(&cert).map_err(|err| {
        Refusal::NotAllowed(format!("the device's certificate is unreadable: {err}"))
    })?;
    let now = now();
    let others = checked(&upload.key_packages, &cert, now, false)?;
    let last = checked(upload.last_resort.as_slice(), &cert, now, true)?;
    let failed = |err| Refusal::Database("store KeyPackages", err);
    let mut tx = db.begin().await.map_err(failed)?;
    // The device's row lock makes its revocation, which deletes its KeyPackages, wait for this
    // upload, or this upload see the revocation.
    let open = sqlx::query("SELECT 1 FROM devices WHERE id = $1::uuid AND NOT revoked FOR SHARE")
        .bind(&device)
        .fetch_optional(&mut *tx)
        .await
        .map_err(failed)?;
    if open.is_none() {
        return Err(Refusal::NotAllowed("the device is revoked".to_owned()));
    }
    for (key_package, expires) in &others {
        sqlx::query(
            "INSERT INTO key_packages (device_id, last_resort, key_package, expires_at)
             VALUES ($1::uuid, false, $2, to_timestamp($3))
             ON CONFLICT (device_id, sha256(key_package)) WHERE NOT last_resort DO NOTHING",
        )
        .bind(&device)
        .bind(key_package)
        .bind(expires)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    }
    if let [(key_package, expires)] = last.as_slice() {
        sqlx::query(
            "INSERT INTO key_packages (device_id, last_resort, key_package, expires_at)
             VALUES ($1::uuid, true, $2, to_timestamp($3))
             ON CONFLICT (device_id) WHERE last_resort DO UPDATE
             SET key_package = EXCLUDED.key_package, expires_at = EXCLUDED.expires_at,
                 handed_out = false",
        )
        .bind(&device)
        .bind(key_package)
        .bind(expires)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    }
    let (held,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM key_packages
         WHERE device_id = $1::uuid AND NOT last_resort AND expires_at > now()",
    )
    .bind(&device)
    .fetch_one(&mut *tx)
    .await
    .map_err(failed)?;
    if held > HELD_LIMIT {
        return Err(Refusal::NotAllowed(format!(
            "the device would hold more than {HELD_LIMIT} unused KeyPackages"
        )));
    }
    tx.commit().await.map_err(failed)?;
    count(db, &device).await
}

/// `POST /key-packages`: hands out one KeyPackage of each device asked for that is a `client`
/// device with a certificate, not revoked, of an account that shares a space with the session's:
/// another one, which it deletes, or once none is left its last-resort one, never an expired one.
pub(crate) async fn hand_out(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let request: Request = parse(&body)?;
    let mut asked: Vec<String> = Vec::new();
    for device in &request.devices {
        let id: DeviceId = device
            .parse()
            .map_err(|err| Refusal::Invalid(format!("devices: {err}")))?;
        let id = id.to_string();
        if !asked.contains(&id) {
            asked.push(id);
        }
    }
    let shared: Vec<(String,)> = sqlx::query_as(
        "SELECT d.id::text FROM devices d
         WHERE d.id = ANY($1::text[]::uuid[]) AND d.kind = 'client' AND d.cert IS NOT NULL
           AND NOT d.revoked
           AND EXISTS (
               SELECT 1 FROM space_access mine
               JOIN space_access theirs ON theirs.space_id = mine.space_id
               WHERE mine.account_id = $2::uuid AND theirs.account_id = d.account_id
           )",
    )
    .bind(&asked)
    .bind(&account)
    .fetch_all(db)
    .await
    .map_err(|err| Refusal::Database("read devices", err))?;
    let mut key_packages = Vec::new();
    for device in asked {
        if !shared.iter().any(|(id,)| *id == device) {
            continue;
        }
        if let Some(key_package) = take(db, &device).await? {
            key_packages.push(Handed {
                key_package: base64(&key_package),
                device,
            });
        }
    }
    Ok(Json(HandedOut { key_packages }).into_response())
}

/// One KeyPackage of `device` for an Add: another one, the one that expires first, deleted as it
/// is taken, so that requests at once never take the same one; once none is left, the last-resort
/// one, marked handed out.
async fn take(db: &PgPool, device: &str) -> Result<Option<Vec<u8>>, Refusal> {
    let failed = |err| Refusal::Database("hand out a KeyPackage", err);
    delete_expired(db, device).await?;
    let other: Option<(Vec<u8>,)> = sqlx::query_as(
        "DELETE FROM key_packages WHERE id = (
             SELECT id FROM key_packages
             WHERE device_id = $1::uuid AND NOT last_resort AND expires_at > now()
             ORDER BY expires_at, id LIMIT 1 FOR UPDATE SKIP LOCKED
         )
         RETURNING key_package",
    )
    .bind(device)
    .fetch_optional(db)
    .await
    .map_err(failed)?;
    if let Some((key_package,)) = other {
        return Ok(Some(key_package));
    }
    let last: Option<(Vec<u8>,)> = sqlx::query_as(
        "UPDATE key_packages SET handed_out = true
         WHERE device_id = $1::uuid AND last_resort AND expires_at > now()
         RETURNING key_package",
    )
    .bind(device)
    .fetch_optional(db)
    .await
    .map_err(failed)?;
    Ok(last.map(|(key_package,)| key_package))
}
