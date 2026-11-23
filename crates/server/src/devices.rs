//! Devices and the account's master key: a signed-in device registers its public keys, the
//! account's first device gives the master public key with its certificate, the account record and
//! the master backup, and the account's sessions read them (`docs/protocol/membership.md`,
//! "Account", "Linking a device").

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use skald_proto::{AccountRecord, CERT_NAME_MAX, Cbor as _, Cert, DeviceId, MasterBackup};

use crate::deletion::{Lock, deleted};
use crate::refusal::{Answer, Refusal, SignedIn, account, answer, parse};
use crate::sign_in::{self, bearer, digest};
use crate::{AppState, link_runs, revocations};

/// The kinds of device that register here: a person's `client` device, and a `service` device a
/// `client` device of the account creates.
const CLIENT: &str = "client";
const SERVICE: &str = "service";

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

/// `Devices` in the spec; with `records` and `deleted`, `SpaceDevices`.
#[derive(Serialize)]
pub(crate) struct Devices {
    devices: Vec<Device>,
    revocations: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    records: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deleted: Vec<String>,
}

impl Devices {
    /// The devices `found`, each an ID and its certificate as the device sent it, and the
    /// `revocations` of their accounts as the server got them.
    pub(crate) fn of(found: Vec<(String, Option<Vec<u8>>)>, revocations: &[(Vec<u8>,)]) -> Self {
        let devices = found
            .into_iter()
            .map(|(device, cert)| Device::new(device, cert))
            .collect();
        let revocations = revocations
            .iter()
            .map(|(revocation,)| base64(revocation))
            .collect();
        Devices {
            devices,
            revocations,
            records: None,
            deleted: Vec::new(),
        }
    }

    /// As [`Devices::of`], with `records`, the account records of the devices' accounts as their
    /// first devices sent them.
    pub(crate) fn with_records(self, records: &[(Vec<u8>,)]) -> Self {
        let records = records.iter().map(|(record,)| base64(record)).collect();
        Devices {
            records: Some(records),
            ..self
        }
    }

    /// As it is, with `deleted`, the IDs of the devices' accounts that are deleted.
    pub(crate) fn with_deleted(self, deleted: Vec<String>) -> Self {
        Devices { deleted, ..self }
    }
}

/// `MasterKey` in the spec.
#[derive(Serialize)]
struct MasterKey {
    master: Option<String>,
    record: Option<String>,
    backup: Option<String>,
}

impl MasterKey {
    fn new(master: Option<&[u8]>, record: Option<&[u8]>, backup: Option<&[u8]>) -> Self {
        MasterKey {
            master: master.map(base64),
            record: record.map(base64),
            backup: backup.map(base64),
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

/// A registration's device, kind, keys and `name`, checked.
struct Checked {
    device: DeviceId,
    kind: &'static str,
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
        let kind = match registration.kind.as_str() {
            CLIENT => CLIENT,
            SERVICE => SERVICE,
            _ => {
                return Err(invalid(
                    "kind: only `client` and `service` devices register",
                ));
            }
        };
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
            kind,
            signing,
            hpke,
            name: registration.name,
        })
    }
}

/// `POST /devices`: registers a `client` or `service` device of the session's account; a device
/// session then lists the `client` one. The same registration again is taken as stored. It
/// certifies nothing, so the devices limit does not count it.
pub(crate) async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let checked = Checked::of(parse(&body)?)?;
        let device = checked.device.to_string();
        let failed = |err| Refusal::Database("register a device", err);
        let mut tx = state.db.begin().await.map_err(failed)?;
        // A deletion of the account holds its lock until it commits: its sessions then end.
        if deleted(&mut tx, &account, Lock::KeyShare)
            .await
            .map_err(failed)?
        {
            return Err(Refusal::NotSignedIn);
        }
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
        .bind(checked.kind)
        .bind(checked.signing.as_slice())
        .bind(checked.hpke.as_slice())
        .bind(checked.name.as_bytes())
        .fetch_optional(&mut *tx)
        .await
        .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        let (created, cert) = stored.ok_or(Refusal::DeviceTaken)?;
        // A device session lists the `client` device it registered: a `service` device it registers
        // is not the session's.
        if checked.kind == CLIENT
            && let Some(Some(token)) = bearer(&headers)
        {
            sqlx::query(
                "UPDATE sessions SET device_id = $1::uuid WHERE kind = 'device' AND token_hash = $2",
            )
            .bind(&device)
            .bind(digest(token))
            .execute(&state.db)
            .await
            .map_err(|err| Refusal::Database("record a session's device", err))?;
        }
        if created && checked.kind == CLIENT {
            let account = account.parse().expect("a signed-in account ID");
            link_runs::registered(&state, &device, account)
                .await
                .map_err(|err| Refusal::Database("tell of a waiting device", err))?;
        }
        taken(created, Device::new(device, cert))
    })
    .await
}

/// `GET /devices`: the session's account's devices, each with its certificate as the device sent it,
/// and the account's revocations.
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
        let revocations = revocations::of_account(&state.db, &account).await?;
        let own = Own {
            devices: Devices::of(found, &revocations),
            web: sign_in::web_origin(state.sign_in.as_deref()),
        };
        Ok(Json(own).into_response())
    })
    .await
}

/// `Devices` in the spec: the account's devices with the server's web address.
#[derive(Serialize)]
struct Own {
    #[serde(flatten)]
    devices: Devices,
    #[serde(skip_serializing_if = "Option::is_none")]
    web: Option<String>,
}

/// `ListedDevice` in the spec.
#[derive(Serialize)]
struct Listed {
    device: String,
    name: String,
    kind: String,
    registered_at: String,
    state: String,
}

/// `ListedDevices` in the spec.
#[derive(Serialize)]
struct ListedDevices {
    devices: Vec<Listed>,
}

/// `GET /account/devices`: the session's account's devices as the web app shows them, without keys
/// or certificates. A device is revoked as the account's stored revocations last marked it.
pub(crate) async fn shown(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    // `to_char` gives RFC 3339 in UTC; a device's name is its UTF-8 bytes.
    let rows: Vec<(String, Vec<u8>, String, String, String)> = sqlx::query_as(
        "SELECT id::text, name, kind,
             to_char(registered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
             CASE WHEN revoked THEN 'revoked' WHEN cert IS NULL THEN 'waiting' ELSE 'linked' END
         FROM devices WHERE account_id = $1::uuid
         ORDER BY registered_at, id",
    )
    .bind(&account)
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the devices", err))?;
    let devices = rows
        .into_iter()
        .map(|(device, name, kind, registered_at, state)| Listed {
            device,
            name: String::from_utf8_lossy(&name).into_owned(),
            kind,
            registered_at,
            state,
        })
        .collect();
    Ok(Json(ListedDevices { devices }).into_response())
}

/// An account's master public key, account record and master backup, each as stored.
type Held = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

/// `GET /account/master`: the session's account's master public key, account record and master
/// backup.
pub(crate) async fn master(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let (master, record, backup): Held =
            sqlx::query_as("SELECT master, record, backup FROM accounts WHERE id = $1::uuid")
                .bind(&account)
                .fetch_one(&state.db)
                .await
                .map_err(|err| Refusal::Database("read the master key", err))?;
        let held = MasterKey::new(master.as_deref(), record.as_deref(), backup.as_deref());
        Ok(Json(held).into_response())
    })
    .await
}

/// `MasterKeyRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MasterKeyRequest {
    master: String,
    cert: String,
    record: String,
    backup: String,
}

/// The account record and the master backup in `request`, as sent, checked against the `master`
/// key and the session's `account`: the record is of `account`, names `master` and `master` signs
/// it; the backup is of the spec's form, which is all a server can check of it.
fn checked_backup(
    request: &MasterKeyRequest,
    master: &[u8; 32],
    account: &str,
) -> Result<(Vec<u8>, Vec<u8>), Refusal> {
    let record = bytes(&request.record, "record")?;
    let decoded = AccountRecord::from_cbor(&record)
        .map_err(|err| Refusal::Invalid(format!("record: not an account record: {err}")))?;
    skald_crypto::check_account(&decoded, master)
        .map_err(|err| Refusal::Invalid(format!("record: {err}")))?;
    if decoded.body().account.to_string() != account {
        return Err(Refusal::Invalid("record: of another account".to_owned()));
    }
    let backup = bytes(&request.backup, "backup")?;
    MasterBackup::from_cbor(&backup)
        .map_err(|err| Refusal::Invalid(format!("backup: not a master backup: {err}")))?;
    Ok((record, backup))
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
/// device, the account record and the master backup while the account has no master key; the same
/// ones again are taken as stored.
pub(crate) async fn give_master(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    answer(async {
        let account = account(&state.db, &headers).await?;
        let request: MasterKeyRequest = parse(&body)?;
        let (master, cert, sent) = checked_cert(&request, &account)?;
        let (record, backup) = checked_backup(&request, &master, &account)?;
        let held = MasterKey::new(Some(&master), Some(&record), Some(&backup));
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
        // One statement, so that of master keys sent at once one is taken, with its certificate,
        // record and backup.
        let stored = sqlx::query(
            "WITH taken AS (
                 UPDATE accounts SET master = $1, record = $5, backup = $6
                 WHERE id = $2::uuid AND master IS NULL RETURNING id
             )
             UPDATE devices SET cert = $3 FROM taken
             WHERE devices.id = $4::uuid AND devices.account_id = taken.id",
        )
        .bind(master.as_slice())
        .bind(&account)
        .bind(&sent)
        .bind(&device)
        .bind(&record)
        .bind(&backup)
        .execute(&state.db)
        .await
        .map_err(|err| Refusal::Database("store the master key", err))?;
        if stored.rows_affected() == 1 {
            return taken(true, held);
        }
        // The account has a master key: taken before, if it is this one with this certificate,
        // record and backup.
        let (same,): (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                 SELECT 1 FROM accounts a JOIN devices d ON d.account_id = a.id
                 WHERE a.id = $1::uuid AND a.master = $2 AND d.id = $3::uuid AND d.cert = $4
                   AND a.record = $5 AND a.backup = $6
             )",
        )
        .bind(&account)
        .bind(master.as_slice())
        .bind(&device)
        .bind(&sent)
        .bind(&record)
        .bind(&backup)
        .fetch_one(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the master key", err))?;
        if !same {
            return Err(Refusal::MasterKeyTaken);
        }
        taken(false, held)
    })
    .await
}
