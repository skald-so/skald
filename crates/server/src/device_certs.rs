//! Certificates a linked device issued: a session of the account stores the certificate of a device
//! of the account registered without one (`docs/protocol/membership.md`, "Link code", step 4 of the
//! new device).

use axum::body::Bytes;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use skald_proto::{Cbor as _, Cert, DeviceId};

use crate::AppState;
use crate::devices::{base64, bytes, taken};
use crate::refusal::{Answer, Refusal, SignedIn, parse};

/// `CertUpload` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Upload {
    cert: String,
}

/// A device's registration: its kind, keys, `name` and certificate.
type Registration = (String, Vec<u8>, Vec<u8>, Vec<u8>, Option<Vec<u8>>);

/// `Device` in the spec.
#[derive(Serialize)]
struct Stored {
    device: String,
    cert: String,
}

/// `PUT /devices/{device}/cert`: stores the certificate of `device`, registered on the session's
/// account without one, once it passes checks 1 and 2 against the account's master key and the
/// certificates the account holds and matches the registration; the same certificate again is
/// taken as stored. With the devices limit, one that would give the account more devices with a
/// certificate, `service` and revoked devices aside, is refused.
pub(crate) async fn store(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(device): Path<String>,
    body: Bytes,
) -> Answer {
    let db = &state.db;
    let id: DeviceId = device.parse().map_err(|_| Refusal::NoDevice)?;
    let device = id.to_string();
    let upload: Upload = parse(&body)?;
    let sent = bytes(&upload.cert, "cert")?;
    let cert =
        Cert::from_cbor(&sent).map_err(|err| Refusal::Cert(format!("not a certificate: {err}")))?;
    let body = cert.body();
    if body.device != id {
        return Err(Refusal::Cert(
            "the certificate is of another device".to_owned(),
        ));
    }
    let registered: Option<Registration> = sqlx::query_as(
        "SELECT kind, signing, hpke, name, cert FROM devices
             WHERE id = $1::uuid AND account_id = $2::uuid",
    )
    .bind(&device)
    .bind(&account)
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a device", err))?;
    let (kind, signing, hpke, name, held) = registered.ok_or(Refusal::NoDevice)?;
    if let Some(held) = held {
        return again(device, &sent, &held);
    }
    // A linked device issues only `client` devices for now: those are the ones that register.
    let matches = body.account.to_string() == account
        && kind == "client"
        && body.kind == skald_proto::DeviceKind::Client
        && signing == body.signing
        && hpke == body.hpke
        && name == body.name.as_bytes();
    if !matches || body.issuer.is_none() {
        return Err(Refusal::Cert(if matches {
            "the master key certifies only the account's first device".to_owned()
        } else {
            "the certificate does not match its device's registration on this account".to_owned()
        }));
    }
    let (master, mut certs) = account_certs(&state, &account).await?;
    let master = master.ok_or_else(|| Refusal::Cert("the account has no master key".to_owned()))?;
    certs.push(cert.clone());
    skald_crypto::check_device(&master, &certs, id)
        .map_err(|err| Refusal::Cert(err.to_string()))?;

    let failed = |err| Refusal::Database("store a certificate", err);
    let mut tx = db.begin().await.map_err(failed)?;
    // The account's row lock orders the certificates of its devices, so of those sent at once the
    // count of each sees the ones before it.
    sqlx::query("SELECT 1 FROM accounts WHERE id = $1::uuid FOR UPDATE")
        .bind(&account)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    // One sent at once with this one may have been stored meanwhile.
    let (held,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT cert FROM devices WHERE id = $1::uuid")
            .bind(&device)
            .fetch_one(&mut *tx)
            .await
            .map_err(failed)?;
    if let Some(held) = held {
        return again(device, &sent, &held);
    }
    if let Some(limit) = state.devices_limit {
        let (certified,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM devices
             WHERE account_id = $1::uuid AND cert IS NOT NULL AND kind <> 'service' AND NOT revoked",
        )
        .bind(&account)
        .fetch_one(&mut *tx)
        .await
        .map_err(failed)?;
        if certified >= i64::from(limit) {
            return Err(Refusal::DevicesLimit(limit));
        }
    }
    let stored = sqlx::query("UPDATE devices SET cert = $1 WHERE id = $2::uuid AND cert IS NULL")
        .bind(&sent)
        .bind(&device)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    if stored.rows_affected() == 1 {
        return taken(true, Stored::of(device, &sent));
    }
    // Another request stored one first: this one again, or another.
    let (held,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT cert FROM devices WHERE id = $1::uuid")
            .bind(&device)
            .fetch_one(db)
            .await
            .map_err(|err| Refusal::Database("read a device", err))?;
    match held {
        Some(held) => again(device, &sent, &held),
        None => Err(Refusal::DeviceCertified),
    }
}

/// The answer to `sent`, a certificate of `device`, which holds `held`: taken again if it is the
/// same, otherwise refused.
fn again(device: String, sent: &[u8], held: &[u8]) -> Answer {
    if held == sent {
        taken(false, Stored::of(device, sent))
    } else {
        Err(Refusal::DeviceCertified)
    }
}

impl Stored {
    fn of(device: String, cert: &[u8]) -> Self {
        Stored {
            device,
            cert: base64(cert),
        }
    }
}

/// The master key of `account` and the certificates its devices hold that decode.
async fn account_certs(
    state: &AppState,
    account: &str,
) -> Result<(Option<[u8; 32]>, Vec<Cert>), Refusal> {
    let (master,): (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT master FROM accounts WHERE id = $1::uuid")
            .bind(account)
            .fetch_one(&state.db)
            .await
            .map_err(|err| Refusal::Database("read the master key", err))?;
    let held: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT cert FROM devices WHERE account_id = $1::uuid AND cert IS NOT NULL")
            .bind(account)
            .fetch_all(&state.db)
            .await
            .map_err(|err| Refusal::Database("read the certificates", err))?;
    let certs = held
        .iter()
        .filter_map(|(cert,)| Cert::from_cbor(cert).ok())
        .collect();
    Ok((master.and_then(|key| key.try_into().ok()), certs))
}
