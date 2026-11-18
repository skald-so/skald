//! The link code relay (`docs/protocol/membership.md`, "Link code"): the server relays a CPace run
//! between sessions of one account, as the bytes it got, and never sees the code. A hello naming a
//! device of the account without a certificate opens the device's run and replaces its open one;
//! later messages go by the run's `session`. A run ends on a `link-abort` from either side, once its
//! device has a certificate and at its expiry; a sweep deletes the runs that ended so.

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use serde::{Deserialize, Serialize};
use skald_proto::{Cbor as _, DeviceId, LinkMessage};
use sqlx::{PgExecutor, PgPool};

use crate::AppState;
use crate::devices::{base64, bytes};
use crate::refusal::{Answer, Refusal, SignedIn, parse};
use crate::sweeps::{self, PAGE, PAUSE};

/// Where a run is open: it has not expired and its device has no certificate. Over `link_runs r`
/// joined with `devices d`.
macro_rules! open {
    () => {
        "r.expires_at > now() AND d.cert IS NULL"
    };
}

/// `LinkHelloRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HelloRequest {
    device: String,
    hello: String,
}

/// `LinkMessageRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageRequest {
    message: String,
}

/// `LinkHello` in the spec.
#[derive(Serialize)]
struct Hello {
    hello: Option<String>,
}

/// `LinkMessages` in the spec.
#[derive(Serialize)]
struct Messages {
    messages: Vec<String>,
}

/// `WaitingDevice` in the spec.
#[derive(Serialize)]
struct Waiting {
    device: String,
    kind: String,
    name: String,
}

/// `WaitingDevices` in the spec.
#[derive(Serialize)]
struct WaitingDevices {
    devices: Vec<Waiting>,
}

/// The link message in `text`, base64url as `field` of the body: `Err` unless it decodes.
fn message(text: &str, field: &str) -> Result<(Vec<u8>, LinkMessage), Refusal> {
    let sent = bytes(text, field)?;
    let message = LinkMessage::from_cbor(&sent)
        .map_err(|err| Refusal::LinkMessage(format!("{field}: not a link message: {err}")))?;
    Ok((sent, message))
}

/// `POST /account/links`: a hello naming a device of the session's account without a certificate
/// opens the device's run, which replaces its open one.
pub(crate) async fn open(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: HelloRequest = parse(&body)?;
    let device: DeviceId = request
        .device
        .parse()
        .map_err(|err| Refusal::Invalid(format!("device: {err}")))?;
    let (hello, LinkMessage::Hello { session, .. }) = message(&request.hello, "hello")? else {
        return Err(Refusal::LinkMessage(
            "hello: not a `link-hello`; a run's later messages go to /account/links/messages"
                .to_owned(),
        ));
    };
    let failed = |err| Refusal::Database("open a link run", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    // The device's lock makes its hellos wait for each other.
    let (certified,): (bool,) = sqlx::query_as(
        "SELECT cert IS NOT NULL FROM devices WHERE id = $1::uuid AND account_id = $2::uuid
         FOR NO KEY UPDATE",
    )
    .bind(device.to_string())
    .bind(&account)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?
    .ok_or(Refusal::NoDevice)?;
    if certified {
        return Err(Refusal::DeviceCertified);
    }
    sqlx::query("DELETE FROM link_runs WHERE device_id = $1::uuid")
        .bind(device.to_string())
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    let opened = sqlx::query(
        "INSERT INTO link_runs (device_id, account_id, session, hello, expires_at)
         VALUES ($1::uuid, $2::uuid, $3, $4, now() + make_interval(secs => $5))
         ON CONFLICT (account_id, session) DO NOTHING",
    )
    .bind(device.to_string())
    .bind(&account)
    .bind(session.as_slice())
    .bind(&hello)
    .bind(state.link_code_lifetime.as_secs_f64())
    .execute(&mut *tx)
    .await
    .map_err(failed)?;
    if opened.rows_affected() == 0 {
        return Err(Refusal::LinkSessionTaken);
    }
    tx.commit().await.map_err(failed)?;
    Ok(StatusCode::CREATED.into_response())
}

/// `GET /account/links/hello`: the latest hello of the account's open runs.
pub(crate) async fn hello(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let latest: Option<(Vec<u8>,)> = sqlx::query_as(concat!(
        "SELECT r.hello FROM link_runs r JOIN devices d ON d.id = r.device_id
         WHERE r.account_id = $1::uuid AND ",
        open!(),
        " ORDER BY r.id DESC LIMIT 1"
    ))
    .bind(&account)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the latest hello", err))?;
    let hello = latest.map(|(hello,)| base64(&hello));
    Ok(Json(Hello { hello }).into_response())
}

/// The open run of `account` with `session`, locked against its end until the transaction ends.
async fn open_run(
    db: impl PgExecutor<'_>,
    account: &str,
    session: &[u8; 16],
) -> Result<i64, Refusal> {
    let run: Option<(i64,)> = sqlx::query_as(concat!(
        "SELECT r.id FROM link_runs r JOIN devices d ON d.id = r.device_id
         WHERE r.account_id = $1::uuid AND r.session = $2 AND ",
        open!(),
        " FOR KEY SHARE OF r"
    ))
    .bind(account)
    .bind(session.as_slice())
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a link run", err))?;
    run.map(|(id,)| id).ok_or(Refusal::NoLinkRun)
}

/// `POST /account/links/messages`: a later message of the account's open run its `session` names;
/// a `link-abort` ends the run.
pub(crate) async fn relay(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: MessageRequest = parse(&body)?;
    let (sent, message) = message(&request.message, "message")?;
    let failed = |err| Refusal::Database("relay a link message", err);
    match message {
        LinkMessage::Hello { .. } => Err(Refusal::LinkMessage(
            "message: a `link-hello` opens a run at /account/links".to_owned(),
        )),
        LinkMessage::Abort { session } => {
            let ended = sqlx::query(concat!(
                "DELETE FROM link_runs r USING devices d
                 WHERE d.id = r.device_id AND r.account_id = $1::uuid AND r.session = $2 AND ",
                open!()
            ))
            .bind(&account)
            .bind(session.as_slice())
            .execute(&state.db)
            .await
            .map_err(failed)?;
            if ended.rows_affected() == 0 {
                return Err(Refusal::NoLinkRun);
            }
            Ok(StatusCode::CREATED.into_response())
        }
        later => {
            let mut tx = state.db.begin().await.map_err(failed)?;
            let run = open_run(&mut *tx, &account, later.session()).await?;
            sqlx::query("INSERT INTO link_messages (run_id, message) VALUES ($1, $2)")
                .bind(run)
                .bind(&sent)
                .execute(&mut *tx)
                .await
                .map_err(failed)?;
            tx.commit().await.map_err(failed)?;
            Ok(StatusCode::CREATED.into_response())
        }
    }
}

/// `GET /account/links/messages/{session}`: the later messages of the account's open run with
/// `session`, in the order the server took them.
pub(crate) async fn messages(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    Path(session): Path<String>,
) -> Answer {
    let session: [u8; 16] = bytes(&session, "session")?
        .try_into()
        .map_err(|_| Refusal::Invalid("session: not 16 bytes".to_owned()))?;
    let run = open_run(&state.db, &account, &session).await?;
    let messages: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT message FROM link_messages WHERE run_id = $1 ORDER BY id")
            .bind(run)
            .fetch_all(&state.db)
            .await
            .map_err(|err| Refusal::Database("read a link run's messages", err))?;
    let messages = messages.iter().map(|(message,)| base64(message)).collect();
    Ok(Json(Messages { messages }).into_response())
}

/// `GET /account/links/waiting`: the account's devices with an open run, by their registration.
pub(crate) async fn waiting(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(concat!(
        "SELECT d.id::text, d.kind, d.name FROM link_runs r JOIN devices d ON d.id = r.device_id
         WHERE r.account_id = $1::uuid AND ",
        open!(),
        " ORDER BY r.id"
    ))
    .bind(&account)
    .fetch_all(&state.db)
    .await
    .map_err(|err| Refusal::Database("read the waiting devices", err))?;
    let devices = rows
        .into_iter()
        .map(|(device, kind, name)| Waiting {
            device,
            kind,
            // A device registers a UTF-8 name.
            name: String::from_utf8_lossy(&name).into_owned(),
        })
        .collect();
    Ok(Json(WaitingDevices { devices }).into_response())
}

/// Looks for runs that ended at their expiry or once their device has a certificate, as
/// [`sweeps::every`] says.
pub(crate) async fn sweep(db: PgPool) {
    sweeps::every("delete ended link runs", || {
        let db = db.clone();
        async move { delete_ended(&db).await }
    })
    .await;
}

/// Deletes the ended runs with their messages, [`PAGE`] runs at a time.
async fn delete_ended(db: &PgPool) -> Result<(), sqlx::Error> {
    loop {
        let deleted = sqlx::query(
            "DELETE FROM link_runs WHERE id IN (
                 SELECT r.id FROM link_runs r JOIN devices d ON d.id = r.device_id
                 WHERE r.expires_at <= now() OR d.cert IS NOT NULL LIMIT $1)",
        )
        .bind(PAGE)
        .execute(db)
        .await?;
        if deleted.rows_affected() == 0 {
            return Ok(());
        }
        // Leaves the database to other work between pages.
        tokio::time::sleep(PAUSE).await;
    }
}

/// How long after its hello a run ends unless a setting shortens it: the code's 5 minutes.
pub const LINK_CODE_LIFETIME: Duration = Duration::from_secs(300);
