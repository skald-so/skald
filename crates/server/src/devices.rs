//! Devices and the account's master key: a signed-in device registers its public keys, the
//! account's first device gives the master public key with its certificate, and the account's
//! sessions read them (`docs/protocol/membership.md`, "Linking a device").

use axum::Json;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use skald_proto::{CERT_NAME_MAX, Cbor as _, Cert, DeviceId};
use sqlx::PgPool;

use crate::sign_in::{not_signed_in, signed_in};
use crate::{AppState, database_failed, problem};

/// The only kind of device that registers here; other kinds come with later stories.
const CLIENT: &str = "client";

/// Why the server refuses a request.
pub(crate) enum Refusal {
    NotSignedIn,
    /// A malformed or invalid body, for the reason it holds.
    Invalid(String),
    /// A certificate the server does not take, for the reason it holds.
    Cert(String),
    DeviceTaken,
    MasterKeyTaken,
    /// No such space, or the session's account does not get it.
    NoSpace,
    /// The session's account may not do this in the space, for the reason it holds.
    NotAllowed(String),
    /// A roster revision the server does not store, for the reason it holds.
    Roster(String),
    SpaceNameTaken,
    /// A roster revision out of order or with a taken number, for the reason it holds.
    RevisionConflict(String),
    /// The database failed at what it holds, such as `read a session`.
    Database(&'static str, sqlx::Error),
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let bad = StatusCode::BAD_REQUEST;
        match self {
            Refusal::NotSignedIn => not_signed_in(),
            Refusal::Invalid(detail) => problem(bad, "invalid-request", "Invalid request", &detail),
            Refusal::Cert(detail) => {
                problem(bad, "invalid-certificate", "Invalid certificate", &detail)
            }
            Refusal::DeviceTaken => problem(
                StatusCode::CONFLICT,
                "device-taken",
                "Device ID taken",
                "The server has this device ID with other keys or name, or on another account.",
            ),
            Refusal::MasterKeyTaken => problem(
                StatusCode::CONFLICT,
                "master-key-taken",
                "The account has a master key",
                "The account has another master key, or took another certificate with it.",
            ),
            Refusal::NoSpace => problem(
                StatusCode::NOT_FOUND,
                "no-space",
                "No such space",
                "No such space, or the session's account does not get it.",
            ),
            Refusal::NotAllowed(detail) => {
                problem(StatusCode::FORBIDDEN, "not-allowed", "Not allowed", &detail)
            }
            Refusal::Roster(detail) => {
                problem(bad, "invalid-roster", "Invalid roster revision", &detail)
            }
            Refusal::SpaceNameTaken => problem(
                StatusCode::CONFLICT,
                "space-name-taken",
                "Space name taken",
                "Another space has this name, or the account's space with it has a revision.",
            ),
            Refusal::RevisionConflict(detail) => problem(
                StatusCode::CONFLICT,
                "revision-conflict",
                "Revision out of order",
                &detail,
            ),
            Refusal::Database(doing, err) => database_failed(doing, err),
        }
    }
}

/// What a handler answers: a response, or why it refuses.
pub(crate) type Answer = Result<Response, Refusal>;

/// The ID of the account of the request's session.
async fn account(db: &PgPool, headers: &HeaderMap) -> Result<String, Refusal> {
    let session = signed_in(db, headers)
        .await
        .map_err(|err| Refusal::Database("read a session", err))?;
    Ok(session.ok_or(Refusal::NotSignedIn)?.account)
}

/// The ID of the account of the request's session, as a handler's argument.
pub(crate) struct SignedIn(pub String);

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = Refusal;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Refusal> {
        Ok(SignedIn(account(&state.db, &parts.headers).await?))
    }
}

/// The JSON `body` as a `T`.
pub(crate) fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, Refusal> {
    serde_json::from_slice(body).map_err(|err| Refusal::Invalid(format!("not a valid body: {err}")))
}

/// The bytes of `text`, base64url without padding; `field` names it in the refusal.
pub(crate) fn bytes(text: &str, field: &str) -> Result<Vec<u8>, Refusal> {
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Refusal::Invalid(format!("{field}: not base64url without padding")))
}

/// `bytes` in base64url without padding.
pub(crate) fn base64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The public key of 32 bytes in `text`, as [`bytes`] reads it.
fn key(text: &str, field: &str) -> Result<[u8; 32], Refusal> {
    bytes(text, field)?
        .try_into()
        .map_err(|_| Refusal::Invalid(format!("{field}: not a key of 32 bytes")))
}

/// `200`, or `201` once `created`, with `body` in JSON.
pub(crate) fn taken(created: bool, body: impl Serialize) -> Answer {
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(body)).into_response())
}

/// Runs a handler's `answer`, which gives a response or why it refuses.
async fn answer(answer: impl Future<Output = Answer>) -> Response {
    answer.await.unwrap_or_else(IntoResponse::into_response)
}

/// `Device` in the spec.
#[derive(Serialize)]
struct Device {
    device: String,
    cert: Option<String>,
}

impl Device {
    fn new(device: String, cert: Option<Vec<u8>>) -> Self {
        Device {
            device,
            cert: cert.as_deref().map(base64),
        }
    }
}

/// `Devices` in the spec.
#[derive(Serialize)]
pub(crate) struct Devices {
    devices: Vec<Device>,
}

impl Devices {
    /// The devices `found`, each an ID and its certificate as the device sent it.
    pub(crate) fn of(found: Vec<(String, Option<Vec<u8>>)>) -> Self {
        let devices = found
            .into_iter()
            .map(|(device, cert)| Device::new(device, cert))
            .collect();
        Devices { devices }
    }
}

/// `MasterKey` in the spec.
#[derive(Serialize)]
struct MasterKey {
    master: Option<String>,
}

impl MasterKey {
    fn new(master: Option<&[u8]>) -> Self {
        MasterKey {
            master: master.map(base64),
        }
    }
}

/// `DeviceRegistration` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    device: String,
    kind: String,
    signing: String,
    hpke: String,
    name: String,
}

/// A registration's device, keys and `name`, checked.
struct Checked {
    device: DeviceId,
    signing: [u8; 32],
    hpke: [u8; 32],
    name: String,
}

impl Checked {
    fn of(registration: Registration) -> Result<Self, Refusal> {
        let invalid = |detail: &str| Refusal::Invalid(detail.to_owned());
        let device: DeviceId = registration
            .device
            .parse()
            .map_err(|err| Refusal::Invalid(format!("device: {err}")))?;
        if registration.kind != CLIENT {
            return Err(invalid("kind: only `client` devices register"));
        }
        let signing = key(&registration.signing, "signing")?;
        let hpke = key(&registration.hpke, "hpke")?;
        if !(1..=CERT_NAME_MAX).contains(&registration.name.len()) {
            return Err(Refusal::Invalid(format!(
                "name: not 1 to {CERT_NAME_MAX} bytes"
            )));
        }
        if !device.is_derived_from(&signing) {
            return Err(invalid("device: not the ID derived from `signing`"));
        }
        Ok(Checked {
            device,
            signing,
            hpke,
            name: registration.name,
        })
    }
}

/// `POST /devices`: registers a device of the session's account; the same registration again is
/// taken as stored.
pub(crate) async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let checked = Checked::of(parse(&body)?)?;
        let device = checked.device.to_string();
        // A new ID is inserted. A taken one is updated to itself, and so returned, only if this
        // registration is the one it has: `xmax` is `0` only in the row of an insert.
        let stored: Option<(bool, Option<Vec<u8>>)> = sqlx::query_as(
            "INSERT INTO devices AS d (id, account_id, kind, signing, hpke, name)
             VALUES ($1::uuid, $2::uuid, $3, $4, $5, $6)
             ON CONFLICT (id) DO UPDATE SET id = d.id
             WHERE (d.account_id, d.kind, d.signing, d.hpke, d.name)
                 = (EXCLUDED.account_id, EXCLUDED.kind, EXCLUDED.signing, EXCLUDED.hpke, EXCLUDED.name)
             RETURNING d.xmax = 0, d.cert",
        )
        .bind(&device)
        .bind(&account)
        .bind(CLIENT)
        .bind(checked.signing.as_slice())
        .bind(checked.hpke.as_slice())
        .bind(checked.name.as_bytes())
        .fetch_optional(&state.db)
        .await
        .map_err(|err| Refusal::Database("register a device", err))?;
        let (created, cert) = stored.ok_or(Refusal::DeviceTaken)?;
        taken(created, Device::new(device, cert))
    })
    .await
}

/// `GET /devices`: the session's account's devices, each with its certificate as the device sent it.
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let found: Vec<(String, Option<Vec<u8>>)> = sqlx::query_as(
            "SELECT id::text, cert FROM devices WHERE account_id = $1::uuid
             ORDER BY registered_at, id",
        )
        .bind(&account)
        .fetch_all(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the devices", err))?;
        Ok(Json(Devices::of(found)).into_response())
    })
    .await
}

/// `GET /account/master`: the session's account's master public key.
pub(crate) async fn master(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let (master,): (Option<Vec<u8>>,) =
            sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid")
                .bind(&account)
                .fetch_one(&state.db)
                .await
                .map_err(|err| Refusal::Database("read the master key", err))?;
        Ok(Json(MasterKey::new(master.as_deref())).into_response())
    })
    .await
}

/// `MasterKeyRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MasterKeyRequest {
    master: String,
    cert: String,
}

/// The master key in `request` and its certificate, decoded and as sent, checked against the key
/// and the session's `account`: it passes the proof check, the master key signs it, and it is of a
/// `client` device of `account`.
fn checked_cert(
    request: &MasterKeyRequest,
    account: &str,
) -> Result<([u8; 32], Cert, Vec<u8>), Refusal> {
    let master = key(&request.master, "master")?;
    let sent = bytes(&request.cert, "cert")?;
    let cert =
        Cert::from_cbor(&sent).map_err(|err| Refusal::Cert(format!("not a certificate: {err}")))?;
    let device = cert.body().device;
    skald_crypto::check_device(&master, std::slice::from_ref(&cert), device)
        .map_err(|err| Refusal::Cert(err.to_string()))?;
    if cert.body().account.to_string() != account {
        return Err(Refusal::Cert(
            "the certificate is of another account".to_owned(),
        ));
    }
    Ok((master, cert, sent))
}

/// `PUT /account/master`: takes the account's master public key with the certificate of its first
/// device while the account has none; the same key and certificate again are taken as stored.
pub(crate) async fn give_master(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let (master, cert, sent) = checked_cert(&parse(&body)?, &account)?;
        let body = cert.body();
        let device = body.device.to_string();
        // The master key issues only `client` devices: the check above holds it.
        let registered = sqlx::query(
            "SELECT 1 FROM devices WHERE id = $1::uuid AND account_id = $2::uuid AND kind = $3
               AND signing = $4 AND hpke = $5 AND name = $6",
        )
        .bind(&device)
        .bind(&account)
        .bind(CLIENT)
        .bind(body.signing.as_slice())
        .bind(body.hpke.as_slice())
        .bind(body.name.as_bytes())
        .fetch_optional(&state.db)
        .await
        .map_err(|err| Refusal::Database("read a device", err))?;
        if registered.is_none() {
            return Err(Refusal::Cert(
                "the certificate does not match its device's registration on this account"
                    .to_owned(),
            ));
        }
        // One statement, so that of master keys sent at once one is taken, with its certificate.
        let stored = sqlx::query(
            "WITH taken AS (
                 UPDATE accounts SET master = $1 WHERE id = $2::uuid AND master IS NULL RETURNING id
             )
             UPDATE devices SET cert = $3 FROM taken
             WHERE devices.id = $4::uuid AND devices.account_id = taken.id",
        )
        .bind(master.as_slice())
        .bind(&account)
        .bind(&sent)
        .bind(&device)
        .execute(&state.db)
        .await
        .map_err(|err| Refusal::Database("store the master key", err))?;
        if stored.rows_affected() == 1 {
            return taken(true, MasterKey::new(Some(&master)));
        }
        // The account has a master key: taken before, if it is this one with this certificate.
        let (same,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                 SELECT 1 FROM accounts a JOIN devices d ON d.account_id = a.id
                 WHERE a.id = $1::uuid AND a.master = $2 AND d.id = $3::uuid AND d.cert = $4
             )",
        )
        .bind(&account)
        .bind(master.as_slice())
        .bind(&device)
        .bind(&sent)
        .fetch_one(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the master key", err))?;
        if !same {
            return Err(Refusal::MasterKeyTaken);
        }
        taken(false, MasterKey::new(Some(&master)))
    })
    .await
}
