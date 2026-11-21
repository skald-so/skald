//! The link code relay (`docs/protocol/membership.md`, "Link code"): the server relays a CPace run
//! between its two sides, as the bytes it got, and never sees the code. A hello naming a device
//! without a certificate opens the device's run and replaces its open one; later messages go by
//! the run's `session`. A run ends on a `link-abort` from either side, once its device has a
//! certificate and at its expiry; a sweep deletes the runs that ended so. A run that opens, or
//! whose device registers, sends each connection of the account's device sessions a `link-notice`,
//! through any instance.
//!
//! - With a session, the hello names a device registered on its account, and the run is the
//!   account's.
//! - Without one, the new device reaches its run by the run's token, and the run is no account's
//!   until a session enters its pairing; the first account to enter it pairs it, unless the device
//!   registered on another account. The run is given as waiting only once its device registers on
//!   an account.

use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse as _;
use serde::{Deserialize, Serialize};
use skald_crypto::LinkPairing;
use skald_proto::{AccountId, Cbor as _, DeviceId, LinkMessage};
use sqlx::{PgExecutor, PgPool, Postgres, Transaction};

use crate::AppState;
use crate::devices::{base64, bytes};
use crate::instances::Event;
use crate::refusal::{Answer, Refusal, SignedIn, parse};
use crate::sign_in::{bearer, digest, random_token};
use crate::sweeps::{self, PAGE, PAUSE};

/// Where a run is open: it has not expired, and its device, if registered, has no certificate and
/// is of the run's account, if it has one. Over `link_runs r`.
macro_rules! open {
    () => {
        "r.expires_at > now() AND NOT EXISTS (
             SELECT 1 FROM devices d
             WHERE d.id = r.device_id AND (d.cert IS NOT NULL OR d.account_id <> r.account_id)
         )"
    };
}

/// Where a run is open and its device registered on its account, or on any account for a run
/// without one: given as waiting. Over `link_runs r` joined with `devices w`, the device.
macro_rules! waiting {
    () => {
        concat!(
            "w.id = r.device_id AND (r.account_id IS NULL OR r.account_id = w.account_id) AND ",
            open!()
        )
    };
}

/// How many later messages a run's token sends.
const RUN_MESSAGES: i32 = 8;
/// The most bytes of a later message from a run's token.
const RUN_MESSAGE_MAX: usize = 4096;
/// How many pairings the server does not hold an account enters within [`WRONG_PAIRING_WINDOW`];
/// then it enters none until the oldest is that old.
const WRONG_PAIRING_LIMIT: i64 = 10;
const WRONG_PAIRING_WINDOW: Duration = Duration::from_secs(900);

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

/// `LinkPairingRequest` in the spec.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingRequest {
    pairing: String,
}

/// `PairedLinkHello` in the spec.
#[derive(Serialize)]
struct PairedHello {
    hello: String,
}

/// `SessionlessLinkRun` in the spec.
#[derive(Serialize)]
struct SessionlessRun {
    pairing: String,
    token: String,
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

/// The device, the hello and its `session` of a `LinkHelloRequest` in `body`, whose later messages
/// go to `later`.
fn hello_request(body: &[u8], later: &str) -> Result<(DeviceId, Vec<u8>, [u8; 16]), Refusal> {
    let request: HelloRequest = parse(body)?;
    let device: DeviceId = request
        .device
        .parse()
        .map_err(|err| Refusal::Invalid(format!("device: {err}")))?;
    let (hello, LinkMessage::Hello { session, .. }) = message(&request.hello, "hello")? else {
        return Err(Refusal::LinkMessage(format!(
            "hello: not a `link-hello`; a run's later messages go to {later}"
        )));
    };
    Ok((device, hello, session))
}

/// The device, the hello and its `session` of a `LinkHelloRequest` in `body`, whose later messages
/// go to `later`, and the transaction that opens its run. The transaction makes the hellos of the
/// device wait for each other, whether it is registered or not, and deletes its open run, which
/// the new one replaces once it commits.
async fn start_run(
    state: &AppState,
    body: &[u8],
    later: &str,
) -> Result<(Transaction<'static, Postgres>, DeviceId, Vec<u8>, [u8; 16]), Refusal> {
    let (device, hello, session) = hello_request(body, later)?;
    let failed = |err| Refusal::Database("open a link run", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("link run {device}"))
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    sqlx::query("DELETE FROM link_runs WHERE device_id = $1::uuid")
        .bind(device.to_string())
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    Ok((tx, device, hello, session))
}

/// `POST /account/links`: a hello naming a device of the session's account without a certificate
/// opens the device's run, which replaces its open one.
pub(crate) async fn open(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let (mut tx, device, hello, session) =
        start_run(&state, &body, "/account/links/messages").await?;
    let failed = |err| Refusal::Database("open a link run", err);
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
    let account_id: AccountId = account.parse().expect("a signed-in account ID");
    state
        .instance
        .tell(&mut tx, &[Event::Linking(account_id)])
        .await
        .map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    state.notices.link(account_id);
    Ok(StatusCode::CREATED.into_response())
}

/// `POST /links`: a hello without a session, naming a device without a certificate, opens the
/// device's run, which replaces its open one, and gives the run's pairing and token.
pub(crate) async fn open_sessionless(State(state): State<AppState>, body: Bytes) -> Answer {
    let (mut tx, device, hello, session) = start_run(&state, &body, "/links/messages").await?;
    let failed = |err| Refusal::Database("open a link run", err);
    let certified: Option<(bool,)> = sqlx::query_as(
        "SELECT cert IS NOT NULL FROM devices WHERE id = $1::uuid FOR NO KEY UPDATE",
    )
    .bind(device.to_string())
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    if certified.is_some_and(|(certified,)| certified) {
        return Err(Refusal::DeviceCertified);
    }
    let token = random_token();
    // A pairing an open or ended run holds is drawn again.
    let pairing = loop {
        let pairing = LinkPairing::generate().to_string();
        let opened = sqlx::query(
            "INSERT INTO link_runs (device_id, session, hello, expires_at, pairing, token_hash)
             VALUES ($1::uuid, $2, $3, now() + make_interval(secs => $4), $5, $6)
             ON CONFLICT (pairing) DO NOTHING",
        )
        .bind(device.to_string())
        .bind(session.as_slice())
        .bind(&hello)
        .bind(state.link_code_lifetime.as_secs_f64())
        .bind(&pairing)
        .bind(digest(&token))
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
        if opened.rows_affected() == 1 {
            break pairing;
        }
    };
    tx.commit().await.map_err(failed)?;
    Ok((StatusCode::CREATED, Json(SessionlessRun { pairing, token })).into_response())
}

/// `POST /account/links/pairing`: the hello of the open run without a session that has the pairing,
/// which pairs the run with the session's account; refused if another account paired it first or
/// its device registered on another account. An account enters at most [`WRONG_PAIRING_LIMIT`]
/// pairings the server does not hold within [`WRONG_PAIRING_WINDOW`].
pub(crate) async fn pair(
    State(state): State<AppState>,
    SignedIn(account): SignedIn,
    body: Bytes,
) -> Answer {
    let request: PairingRequest = parse(&body)?;
    let window = WRONG_PAIRING_WINDOW.as_secs_f64();
    let failed = |err| Refusal::Database("pair a link run", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    // The account's lock makes its pairings wait for each other, so none slips past the limit.
    sqlx::query("SELECT 1 FROM accounts WHERE id = $1::uuid FOR NO KEY UPDATE")
        .bind(&account)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
    let (wrong,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM wrong_pairings
         WHERE account_id = $1::uuid AND entered_at > now() - make_interval(secs => $2)",
    )
    .bind(&account)
    .bind(window)
    .fetch_one(&mut *tx)
    .await
    .map_err(failed)?;
    if wrong >= WRONG_PAIRING_LIMIT {
        return Err(Refusal::TooManyPairings);
    }
    // The run, the account it is paired with, its `session` and hello, and its device's account.
    type Found = (i64, Option<String>, Vec<u8>, Vec<u8>, Option<String>);
    let found: Option<Found> = sqlx::query_as(concat!(
        "SELECT r.id, r.account_id::text, r.session, r.hello, w.account_id::text
             FROM link_runs r LEFT JOIN devices w ON w.id = r.device_id
             WHERE r.pairing = $1 AND ",
        open!(),
        " FOR NO KEY UPDATE OF r"
    ))
    .bind(&request.pairing)
    .fetch_optional(&mut *tx)
    .await
    .map_err(failed)?;
    let ours = |of: &Option<String>| of.as_ref().is_none_or(|of| *of == account);
    let paired = match found {
        Some((run, paired, session, hello, registered)) if ours(&paired) && ours(&registered) => {
            if paired.is_none() {
                let taken = sqlx::query(
                    "UPDATE link_runs SET account_id = $2::uuid WHERE id = $1 AND NOT EXISTS (
                         SELECT 1 FROM link_runs WHERE account_id = $2::uuid AND session = $3
                     )",
                )
                .bind(run)
                .bind(&account)
                .bind(&session)
                .execute(&mut *tx)
                .await
                .map_err(failed)?;
                if taken.rows_affected() == 0 {
                    return Err(Refusal::LinkSessionTaken);
                }
            }
            Some(hello)
        }
        _ => None,
    };
    let Some(hello) = paired else {
        sqlx::query(
            "WITH old AS (
                 DELETE FROM wrong_pairings
                 WHERE account_id = $1::uuid AND entered_at <= now() - make_interval(secs => $2)
             )
             INSERT INTO wrong_pairings (account_id) VALUES ($1::uuid)",
        )
        .bind(&account)
        .bind(window)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
        tx.commit().await.map_err(failed)?;
        return Err(Refusal::NoPairing);
    };
    tx.commit().await.map_err(failed)?;
    Ok(Json(PairedHello {
        hello: base64(&hello),
    })
    .into_response())
}

/// The open run without a session whose token is the request's bearer token, locked against its
/// end until the transaction ends: its ID and `session`.
async fn token_run(
    db: impl PgExecutor<'_>,
    headers: &HeaderMap,
) -> Result<(i64, Vec<u8>), Refusal> {
    let Some(Some(token)) = bearer(headers) else {
        return Err(Refusal::NotSignedIn);
    };
    let run: Option<(i64, Vec<u8>)> = sqlx::query_as(concat!(
        "SELECT r.id, r.session FROM link_runs r WHERE r.token_hash = $1 AND ",
        open!(),
        " FOR NO KEY UPDATE OF r"
    ))
    .bind(digest(token))
    .fetch_optional(db)
    .await
    .map_err(|err| Refusal::Database("read a link run", err))?;
    run.ok_or(Refusal::NotSignedIn)
}

/// `GET /links/messages`: the later messages of the open run of the request's run token, in the
/// order the server took them.
pub(crate) async fn run_messages(State(state): State<AppState>, headers: HeaderMap) -> Answer {
    let (run, _) = token_run(&state.db, &headers).await?;
    stored_messages(&state.db, run).await
}

/// `POST /links/messages`: a later message of the open run of the request's run token, from its new
/// device; a `link-abort` ends the run. The token sends at most [`RUN_MESSAGES`] later messages of
/// at most [`RUN_MESSAGE_MAX`] bytes each.
pub(crate) async fn run_relay(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Answer {
    let request: MessageRequest = parse(&body)?;
    let failed = |err| Refusal::Database("relay a link message", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    let (run, session) = token_run(&mut *tx, &headers).await?;
    let (sent, message) = message(&request.message, "message")?;
    if sent.len() > RUN_MESSAGE_MAX {
        return Err(Refusal::LinkMessage(format!(
            "message: more than {RUN_MESSAGE_MAX} bytes"
        )));
    }
    if let LinkMessage::Hello { .. } = message {
        return Err(Refusal::LinkMessage(
            "message: a `link-hello` opens a run at /links".to_owned(),
        ));
    }
    if message.session().as_slice() != session {
        return Err(Refusal::NoLinkRun);
    }
    if let LinkMessage::Abort { .. } = message {
        sqlx::query("DELETE FROM link_runs WHERE id = $1")
            .bind(run)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
    } else {
        let counted =
            sqlx::query("UPDATE link_runs SET sent = sent + 1 WHERE id = $1 AND sent < $2")
                .bind(run)
                .bind(RUN_MESSAGES)
                .execute(&mut *tx)
                .await
                .map_err(failed)?;
        if counted.rows_affected() == 0 {
            return Err(Refusal::TooManyLinkMessages(RUN_MESSAGES));
        }
        sqlx::query("INSERT INTO link_messages (run_id, message) VALUES ($1, $2)")
            .bind(run)
            .bind(&sent)
            .execute(&mut *tx)
            .await
            .map_err(failed)?;
    }
    tx.commit().await.map_err(failed)?;
    Ok(StatusCode::CREATED.into_response())
}

/// `GET /account/links/hello`: the latest hello of the account's open runs with a session.
pub(crate) async fn hello(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let latest: Option<(Vec<u8>,)> = sqlx::query_as(concat!(
        "SELECT r.hello FROM link_runs r
         WHERE r.account_id = $1::uuid AND r.pairing IS NULL AND ",
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
        "SELECT r.id FROM link_runs r
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
                "DELETE FROM link_runs r WHERE r.account_id = $1::uuid AND r.session = $2 AND ",
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
    stored_messages(&state.db, run).await
}

/// The later messages of `run`, in the order the server took them.
async fn stored_messages(db: &PgPool, run: i64) -> Answer {
    let messages: Vec<(Vec<u8>,)> =
        sqlx::query_as("SELECT message FROM link_messages WHERE run_id = $1 ORDER BY id")
            .bind(run)
            .fetch_all(db)
            .await
            .map_err(|err| Refusal::Database("read a link run's messages", err))?;
    let messages = messages.iter().map(|(message,)| base64(message)).collect();
    Ok(Json(Messages { messages }).into_response())
}

/// `GET /account/links/waiting`: the account's devices with an open run, by their registration.
pub(crate) async fn waiting(State(state): State<AppState>, SignedIn(account): SignedIn) -> Answer {
    let rows: Vec<(String, String, Vec<u8>)> = sqlx::query_as(concat!(
        "SELECT w.id::text, w.kind, w.name FROM link_runs r JOIN devices w ON ",
        waiting!(),
        " WHERE w.account_id = $1::uuid ORDER BY r.id"
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

/// Whether `account` has an open run of a link code that opened since the previous call for
/// `device`, any open run at the first call; records the latest, so that the next call gives only
/// a later one.
pub(crate) async fn since_last_answer(
    db: &PgPool,
    device: DeviceId,
    account: AccountId,
) -> Result<bool, sqlx::Error> {
    let found: Option<(i64,)> = sqlx::query_as(concat!(
        "WITH latest AS (
             SELECT max(r.id) AS run_id FROM link_runs r JOIN devices w ON ",
        waiting!(),
        "
             WHERE w.account_id = $2::uuid
         )
         INSERT INTO long_poll_link_runs (device_id, run_id)
         SELECT $1::uuid, run_id FROM latest WHERE run_id IS NOT NULL
         ON CONFLICT (device_id) DO UPDATE SET run_id = EXCLUDED.run_id
         WHERE long_poll_link_runs.run_id < EXCLUDED.run_id
         RETURNING run_id"
    ))
    .bind(device.to_string())
    .bind(account.to_string())
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Sends a `link-notice` to the connections of `account`'s device sessions, through any instance, if
/// `device`, just registered on it, has an open run without a session that is now given as waiting.
pub(crate) async fn registered(
    state: &AppState,
    device: &str,
    account: AccountId,
) -> Result<(), sqlx::Error> {
    let (waiting,): (bool,) = sqlx::query_as(concat!(
        "SELECT EXISTS (SELECT 1 FROM link_runs r JOIN devices w ON ",
        waiting!(),
        " WHERE r.device_id = $1::uuid AND r.pairing IS NOT NULL)"
    ))
    .bind(device)
    .fetch_one(&state.db)
    .await?;
    if waiting {
        let mut conn = state.db.acquire().await?;
        state
            .instance
            .tell(&mut conn, &[Event::Linking(account)])
            .await?;
        state.notices.link(account);
    }
    Ok(())
}

/// The accounts of `accounts`, account IDs in their text form, with an open run.
pub(crate) async fn linking(
    db: &PgPool,
    accounts: &[String],
) -> Result<Vec<AccountId>, sqlx::Error> {
    let found: Vec<(String,)> = sqlx::query_as(concat!(
        "SELECT DISTINCT w.account_id::text FROM link_runs r JOIN devices w ON ",
        waiting!(),
        " WHERE w.account_id = ANY($1::text[]::uuid[])"
    ))
    .bind(accounts)
    .fetch_all(db)
    .await?;
    Ok(found
        .into_iter()
        .map(|(account,)| account.parse().expect("a stored account ID"))
        .collect())
}

/// Looks for runs that ended at their expiry, once their device has a certificate or registered on
/// another account, as [`sweeps::every`] says.
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
        let deleted = sqlx::query(concat!(
            "DELETE FROM link_runs WHERE id IN (
                 SELECT r.id FROM link_runs r WHERE NOT (",
            open!(),
            ") LIMIT $1)"
        ))
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
