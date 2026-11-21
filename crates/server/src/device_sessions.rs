//! A device's session by its key (`docs/protocol/membership.md`, "Device key session"): a `client`
//! device with a certificate signs a challenge with its signing key and gets a device session of
//! its account with no sign-in. A device the server holds no certificate of sends one, which the
//! server stores with the checks and the devices limit of `PUT /devices/{device}/cert`, registering
//! a device it does not have. A device has one such session at a time; it ends as any device
//! session does, and when its device is revoked.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use skald_crypto::DeviceCheckError;
use skald_proto::{Cbor as _, Cert, DeviceId, DeviceKind, Revocation};
use sqlx::{Postgres, Transaction};

use crate::AppState;
use crate::connections::CHALLENGE_LIFETIME;
use crate::device_certs::{check_on_account, lock_account, within_limit};
use crate::devices::{base64, bytes};
use crate::refusal::{Answer, Refusal, parse};
use crate::sessions::{close_ended, tell_ended};
use crate::sign_in::{digest, random_token};
use crate::spaces::certs;

/// The label of a device's signature over its challenge.
const LABEL: &str = "skald device session";

/// `Challenge` in the spec.
#[derive(Serialize)]
struct Challenge {
    challenge: String,
}

/// `DeviceKeySessionRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    challenge: String,
    signature: String,
    cert: Option<String>,
}

/// `DeviceKeySession` in the spec.
#[derive(Serialize)]
struct KeySession {
    token: String,
}

/// A device as the server holds it: its account, kind, keys, `name`, certificate and whether it is
/// revoked.
type Held = (
    String,
    String,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Option<Vec<u8>>,
    bool,
);

/// The device ID in the path.
fn device_id(device: &str) -> Result<DeviceId, Refusal> {
    device
        .parse()
        .map_err(|err| Refusal::Invalid(format!("device: {err}")))
}

/// `POST /devices/{device}/challenge`: a new challenge for any device ID, for
/// [`CHALLENGE_LIFETIME`].
pub(crate) async fn challenge(State(state): State<AppState>, Path(device): Path<String>) -> Answer {
    let device = device_id(&device)?;
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge).expect("the operating system's random generator failed");
    sqlx::query(
        "WITH expired AS (DELETE FROM device_challenges WHERE expires_at <= now())
         INSERT INTO device_challenges (challenge_hash, device_id, expires_at)
         VALUES ($1, $2::uuid, now() + make_interval(secs => $3))",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .bind(device.to_string())
    .bind(CHALLENGE_LIFETIME.as_secs_f64())
    .execute(&state.db)
    .await
    .map_err(|err| Refusal::Database("issue a challenge", err))?;
    Ok(Json(Challenge {
        challenge: base64(&challenge),
    })
    .into_response())
}

/// `POST /devices/{device}/session`: a device session of the device's account, for a signature by
/// its signing key over a challenge the server issued for it, with the certificate it holds or
/// stores. The challenge is used up whatever the answer; the same request again gets a new session
/// in place of the first while the challenge lasts.
pub(crate) async fn start(
    State(state): State<AppState>,
    Path(device): Path<String>,
    body: Bytes,
) -> Answer {
    let id = device_id(&device)?;
    let device = id.to_string();
    let request: Request = parse(&body)?;
    let challenge = bytes(&request.challenge, "challenge")?;
    let signature: [u8; 64] = bytes(&request.signature, "signature")?
        .try_into()
        .map_err(|_| Refusal::Invalid("signature: not 64 bytes".to_owned()))?;
    let sent = request
        .cert
        .as_deref()
        .map(|cert| bytes(cert, "cert"))
        .transpose()?;
    let again = take_challenge(&state, &challenge, &device, &Sha256::digest(&body)).await?;

    let held: Option<Held> = sqlx::query_as(
        "SELECT account_id::text, kind, signing, hpke, name, cert, revoked FROM devices
         WHERE id = $1::uuid",
    )
    .bind(&device)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let held_cert = held.as_ref().and_then(|held| held.5.clone());
    let cert_bytes = match (&held_cert, &sent) {
        (Some(held), Some(sent)) if held != sent => return Err(Refusal::DeviceCertified),
        (Some(held), _) => held.clone(),
        (None, Some(sent)) => sent.clone(),
        (None, None) => return Err(Refusal::NoDevice),
    };
    let cert = Cert::from_cbor(&cert_bytes)
        .map_err(|err| Refusal::Cert(format!("not a certificate: {err}")))?;
    let body = cert.body();
    if body.device != id || !id.is_derived_from(&body.signing) {
        return Err(Refusal::Cert(
            "the certificate is of another device".to_owned(),
        ));
    }
    if body.kind != DeviceKind::Client {
        return Err(Refusal::Cert(
            "only a `client` device gets a session by its key".to_owned(),
        ));
    }
    let origin = state
        .sign_in
        .as_ref()
        .map(|sign_in| sign_in.public_url.origin())
        .ok_or(Refusal::SignatureRefused)?;
    skald_crypto::verify(
        &body.signing,
        LABEL,
        &[&challenge, origin.as_bytes()],
        &signature,
    )
    .map_err(|_| Refusal::SignatureRefused)?;
    let account = body.account.to_string();
    if held_cert.is_none() {
        check_new(&state, id, &cert, held.as_ref()).await?;
    }

    let failed = |err| Refusal::Database("start a device session", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    // The account's row lock orders the certificates of its devices, as `PUT
    // /devices/{device}/cert` takes it, and its revocations, which wait for the session.
    lock_account(&mut tx, &account).await.map_err(failed)?;
    unrevoked(&mut tx, &account, held_cert.is_none().then_some(&cert), id).await?;
    if held_cert.is_none() {
        store(&state, &mut tx, &cert_bytes, &cert).await?;
    }
    // The device's lock makes a revocation wait for the session, which it then ends, or the
    // session wait for the revocation.
    let (owner, stored, revoked): (String, Option<Vec<u8>>, bool) = sqlx::query_as(
        "SELECT account_id::text, cert, revoked FROM devices WHERE id = $1::uuid FOR SHARE",
    )
    .bind(&device)
    .fetch_one(&mut *tx)
    .await
    .map_err(failed)?;
    if owner != account || stored.as_deref() != Some(cert_bytes.as_slice()) {
        return Err(Refusal::DeviceCertified);
    }
    if revoked {
        return Err(Refusal::DeviceRevoked);
    }
    let ended: Vec<(Vec<u8>,)> =
        sqlx::query_as("DELETE FROM sessions WHERE key_device = $1::uuid RETURNING token_hash")
            .bind(&device)
            .fetch_all(&mut *tx)
            .await
            .map_err(failed)?;
    let token = random_token();
    sqlx::query(
        "INSERT INTO sessions (token_hash, account_id, kind, device_id, key_device)
         VALUES ($1, $2::uuid, 'device', $3::uuid, $3::uuid)",
    )
    .bind(digest(&token))
    .bind(&account)
    .bind(&device)
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    tell_ended(&state, &mut tx, &ended).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    close_ended(&state, &ended);
    let status = if again {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(KeySession { token })).into_response())
}

/// Uses up `challenge`, which must be one the server issued for `device` and has not expired, for
/// the request whose SHA-256 is `request`: whether that request used it before. Refused if another
/// request did.
async fn take_challenge(
    state: &AppState,
    challenge: &[u8],
    device: &str,
    request: &[u8],
) -> Result<bool, Refusal> {
    let taken: Option<(bool,)> = sqlx::query_as(
        "WITH c AS (
             SELECT challenge_hash, request FROM device_challenges
             WHERE challenge_hash = $1 AND device_id = $2::uuid AND expires_at > now()
             FOR UPDATE
         )
         UPDATE device_challenges u SET request = $3 FROM c
         WHERE u.challenge_hash = c.challenge_hash AND (c.request IS NULL OR c.request = $3)
         RETURNING c.request IS NOT NULL",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .bind(device)
    .bind(request)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Refusal::Database("take a challenge", err))?;
    taken.map(|(again,)| again).ok_or(Refusal::ChallengeRefused)
}

/// Checks `cert`, sent for device `id`, which the server holds as `held` without a certificate or
/// not at all: issued by a device, not the master key; passes checks 1 and 2 against its account's
/// master key and the certificates its devices hold; and matches the registration, if any.
async fn check_new(
    state: &AppState,
    id: DeviceId,
    cert: &Cert,
    held: Option<&Held>,
) -> Result<(), Refusal> {
    let body = cert.body();
    if body.issuer.is_none() {
        return Err(Refusal::Cert(
            "a device of the account issues it, not the master key".to_owned(),
        ));
    }
    let account = body.account.to_string();
    if let Some((owner, kind, signing, hpke, name, _, _)) = held {
        let matches = *owner == account
            && kind == "client"
            && *signing == body.signing
            && *hpke == body.hpke
            && name == body.name.as_bytes();
        if !matches {
            return Err(Refusal::Cert(
                "the certificate does not match its device's registration".to_owned(),
            ));
        }
    }
    check_on_account(state, &account, cert, id).await
}

/// Refuses device `id` if checks 3 and 4 of "Device checks" revoke it, against the master key of
/// `account`, the certificates its devices hold with `sent` if given, and its stored revocations,
/// read in `tx`.
async fn unrevoked(
    tx: &mut Transaction<'_, Postgres>,
    account: &str,
    sent: Option<&Cert>,
    id: DeviceId,
) -> Result<(), Refusal> {
    let failed = |err| Refusal::Database("read the revocations", err);
    let (master,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid")
            .bind(account)
            .fetch_one(&mut **tx)
            .await
            .map_err(failed)?;
    let master: [u8; 32] = master
        .and_then(|master| master.try_into().ok())
        .ok_or_else(|| Refusal::Cert("the account has no master key".to_owned()))?;
    let mut certs = certs(&mut **tx, &[account.to_owned()]).await?;
    certs.extend(sent.cloned());
    let stored: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT revocation FROM revocations WHERE account_id = $1::uuid")
            .bind(account)
            .fetch_all(&mut **tx)
            .await
            .map_err(failed)?;
    let revocations: Vec<Revocation> = stored
        .iter()
        .map(|(bytes,)| Revocation::from_cbor(bytes).expect("a stored revocation decodes"))
        .collect();
    match skald_crypto::check_valid_device(&master, &certs, &revocations, id) {
        Ok(_) => Ok(()),
        Err(DeviceCheckError::Revoked { .. }) => Err(Refusal::DeviceRevoked),
        Err(err) => Err(Refusal::Cert(err.to_string())),
    }
}

/// Stores `cert`, the checked certificate `sent`, in `tx` within the devices limit, registering its
/// device if the server does not have it. One stored meanwhile is left for the caller to compare.
async fn store(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    sent: &[u8],
    cert: &Cert,
) -> Result<(), Refusal> {
    let failed = |err| Refusal::Database("store a certificate", err);
    let body = cert.body();
    let account = body.account.to_string();
    let held: Option<(Option<Vec<u8>>,)> =
        sqlx::query_as("SELECT cert FROM devices WHERE id = $1::uuid")
            .bind(body.device.to_string())
            .fetch_optional(&mut **tx)
            .await
            .map_err(failed)?;
    if held.is_some_and(|(cert,)| cert.is_some()) {
        return Ok(());
    }
    within_limit(tx, state.devices_limit, &account).await?;
    // A registration with the certificate's keys and `name`, as `POST /devices` stores it.
    sqlx::query(
        "INSERT INTO devices AS d (id, account_id, kind, signing, hpke, name, cert)
         VALUES ($1::uuid, $2::uuid, 'client', $3, $4, $5, $6)
         ON CONFLICT (id) DO UPDATE SET cert = EXCLUDED.cert
         WHERE d.cert IS NULL AND (d.account_id, d.kind, d.signing, d.hpke, d.name)
             = (EXCLUDED.account_id, EXCLUDED.kind, EXCLUDED.signing, EXCLUDED.hpke, EXCLUDED.name)",
    )
    .bind(body.device.to_string())
    .bind(&account)
    .bind(body.signing.as_slice())
    .bind(body.hpke.as_slice())
    .bind(body.name.as_bytes())
    .bind(sent)
    .execute(&mut **tx)
    .await
    .map_err(failed)?;
    Ok(())
}
