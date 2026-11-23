//! The account's sessions: a web session lists them and ends any of them, from a page of this
//! server; any session signs itself out. An ended session is refused at once, and its connections
//! close on every instance. A web session expires [`WEB_LIFETIME`] after its sign-in; a device
//! session lasts until it ends.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use sqlx::postgres::PgArguments;
use sqlx::query::QueryAs;
use sqlx::{Postgres, Transaction};

use crate::instances::Event;
use crate::refusal::{Refusal, answer};
use crate::sign_in::{
    ProviderKind, Session, bearer, digest, from_this_server, not_signed_in, signed_in,
};
use crate::{AppState, database_failed};

/// How long a web session lasts after its sign-in, and its cookie with it.
pub(crate) const WEB_LIFETIME: Duration = Duration::from_secs(30 * 86_400);

/// `ListedSession` in the spec.
#[derive(Serialize)]
struct Listed {
    session: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sign_in_method: Option<ProviderKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    browser: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_name: Option<String>,
    started_at: String,
    last_used_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    current: bool,
}

/// `Sessions` in the spec.
#[derive(Serialize)]
struct Sessions {
    sessions: Vec<Listed>,
}

/// A session of the account as the database lists it.
type Row = (
    String,
    String,
    Option<String>,
    Option<String>,
    Option<Vec<u8>>,
    Option<String>,
    String,
    String,
    Option<String>,
    bool,
);

/// What only a web session does: lists or ends the account's sessions.
const LISTS_OR_ENDS: &str = "lists or ends the account's sessions";

/// The web session of the request's cookie; a device session is refused, as only a web session
/// `does` this, such as [`LISTS_OR_ENDS`].
pub(crate) async fn web_session(
    state: &AppState,
    headers: &HeaderMap,
    does: &str,
) -> Result<Session, Refusal> {
    let session = signed_in(&state.db, headers)
        .await
        .map_err(|err| Refusal::Database("read a session", err))?
        .ok_or(Refusal::NotSignedIn)?;
    // Only web sessions sign in with a method.
    if session.sign_in_method.is_none() {
        return Err(Refusal::NotAllowed(format!("Only a web session {does}.")));
    }
    Ok(session)
}

/// The web session of the request's cookie, as [`web_session`] takes it for what it `does`, if the
/// request comes from a page of this server.
pub(crate) async fn web_session_from_page(
    state: &AppState,
    headers: &HeaderMap,
    does: &str,
) -> Result<Session, Refusal> {
    let session = web_session(state, headers, does).await?;
    from_page(state, headers)?;
    Ok(session)
}

/// Refuses `session` unless its sign-in is more recent than `age`, read on `conn`: with
/// `SignInTooOld` when it is older, `NotSignedIn` when the session has ended. A database failure is
/// one at `doing`.
pub(crate) async fn recent(
    conn: &mut sqlx::PgConnection,
    session: &Session,
    age: Duration,
    doing: &'static str,
) -> Result<(), Refusal> {
    let recent: Option<(bool,)> = sqlx::query_as(
        "SELECT created_at > now() - make_interval(secs => $2) FROM sessions
         WHERE token_hash = $1",
    )
    .bind(&session.token_hash)
    .bind(age.as_secs_f64())
    .fetch_optional(conn)
    .await
    .map_err(|err| Refusal::Database(doing, err))?;
    match recent {
        None => Err(Refusal::NotSignedIn),
        Some((false,)) => Err(Refusal::SignInTooOld),
        Some((true,)) => Ok(()),
    }
}

/// Refuses the request unless it comes from a page of this server, as its `Origin` says.
pub(crate) fn from_page(state: &AppState, headers: &HeaderMap) -> Result<(), Refusal> {
    let from_page = state
        .sign_in
        .as_deref()
        .is_some_and(|settings| from_this_server(settings, headers));
    if !from_page {
        return Err(Refusal::NotAllowed(
            "The request does not come from a page of this server.".to_owned(),
        ));
    }
    Ok(())
}

/// `GET /account/sessions`: every session of the web session's account that has not ended.
pub(crate) async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let session = web_session(&state, &headers, LISTS_OR_ENDS).await?;
        // `to_char` gives RFC 3339 in UTC; a device's name is its UTF-8 bytes.
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT s.id::text, s.kind, s.browser, s.os, d.name, s.method_provider,
                 to_char(s.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
                 to_char(s.last_used_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'),
                 CASE WHEN s.kind = 'web' THEN to_char(
                     (s.created_at + make_interval(secs => $3)) AT TIME ZONE 'UTC',
                     'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') END,
                 s.token_hash = $2
             FROM sessions s LEFT JOIN devices d ON d.id = s.device_id
             WHERE s.account_id = $1::uuid
                 AND (s.kind = 'device' OR s.created_at > now() - make_interval(secs => $3))
             ORDER BY s.created_at DESC, s.id",
        )
        .bind(&session.account)
        .bind(&session.token_hash)
        .bind(WEB_LIFETIME.as_secs_f64())
        .fetch_all(&state.db)
        .await
        .map_err(|err| Refusal::Database("read the sessions", err))?;
        let sessions = rows
            .into_iter()
            .map(
                |(id, kind, browser, os, device, method, started, used, expires, current)| Listed {
                    session: id,
                    kind,
                    sign_in_method: method.as_deref().map(ProviderKind::from_key),
                    browser,
                    os,
                    device_name: device.map(|name| String::from_utf8_lossy(&name).into_owned()),
                    started_at: started,
                    last_used_at: used,
                    expires_at: expires,
                    current,
                },
            )
            .collect();
        Ok(Json(Sessions { sessions }).into_response())
    })
    .await
}

/// `DELETE /account/sessions/{session}`: ends that session of the web session's account.
pub(crate) async fn end_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    answer(async {
        let session = web_session_from_page(&state, &headers, LISTS_OR_ENDS).await?;
        // As text: an ID that is no UUID is no session.
        let ended = end(
            &state,
            sqlx::query_as(
                "DELETE FROM sessions WHERE account_id = $1::uuid AND id::text = lower($2)
                     AND (kind = 'device' OR created_at > now() - make_interval(secs => $3))
                 RETURNING token_hash",
            )
            .bind(&session.account)
            .bind(&id)
            .bind(WEB_LIFETIME.as_secs_f64()),
        )
        .await?;
        if ended == 0 {
            return Err(Refusal::NoSession);
        }
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

/// `POST /account/sessions/end-others`: ends every session of the web session's account but itself.
pub(crate) async fn end_others(State(state): State<AppState>, headers: HeaderMap) -> Response {
    answer(async {
        let session = web_session_from_page(&state, &headers, LISTS_OR_ENDS).await?;
        end(
            &state,
            sqlx::query_as(
                "DELETE FROM sessions WHERE account_id = $1::uuid AND token_hash <> $2
                 RETURNING token_hash",
            )
            .bind(&session.account)
            .bind(&session.token_hash),
        )
        .await?;
        Ok(StatusCode::NO_CONTENT.into_response())
    })
    .await
}

/// `DELETE /session`: signs the request's session out: the device session of its bearer token, or
/// the web session of its cookie from a page of this server, which also removes the cookie.
pub(crate) async fn sign_out(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let ended = match bearer(&headers) {
        Some(Some(token)) => {
            let ended = end(
                &state,
                sqlx::query_as(
                    "DELETE FROM sessions WHERE kind = 'device' AND token_hash = $1
                     RETURNING token_hash",
                )
                .bind(digest(token)),
            )
            .await;
            ended.map(|ended| (ended, None))
        }
        Some(None) => return not_signed_in(),
        None => match web_session_from_page(&state, &headers, LISTS_OR_ENDS).await {
            Ok(session) => {
                let ended = end(
                    &state,
                    sqlx::query_as(
                        "DELETE FROM sessions WHERE token_hash = $1 RETURNING token_hash",
                    )
                    .bind(&session.token_hash),
                )
                .await;
                let settings = state.sign_in.as_deref().expect("a web session signs in");
                ended.map(|ended| (ended, Some(settings.session_cookie("", 0))))
            }
            // A cookie that is not from a page of this server signs nothing out.
            Err(Refusal::Database(doing, err)) => return database_failed(doing, err),
            Err(_) => return not_signed_in(),
        },
    };
    match ended {
        Ok((0, _)) => not_signed_in(),
        Ok((_, cookie)) => {
            let mut response = StatusCode::NO_CONTENT.into_response();
            if let Some(cookie) = cookie {
                response.headers_mut().insert(header::SET_COOKIE, cookie);
            }
            response
        }
        Err(refusal) => refusal.into_response(),
    }
}

/// Ends the sessions that `delete`, a `DELETE` that returns their tokens' SHA-256s, removes: tells
/// the other instances in its transaction, then closes this instance's connections of them. Gives
/// how many ended.
async fn end(
    state: &AppState,
    delete: QueryAs<'_, Postgres, (Vec<u8>,), PgArguments>,
) -> Result<usize, Refusal> {
    let failed = |err| Refusal::Database("end a session", err);
    let mut tx = state.db.begin().await.map_err(failed)?;
    let ended = delete.fetch_all(&mut *tx).await.map_err(failed)?;
    tell_ended(state, &mut tx, &ended).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    close_ended(state, &ended);
    Ok(ended.len())
}

/// Tells the other instances in `tx`, which removes the sessions whose tokens' SHA-256s are
/// `ended`, that they ended; once `tx` commits, [`close_ended`] closes this instance's connections
/// of them.
pub(crate) async fn tell_ended(
    state: &AppState,
    tx: &mut Transaction<'_, Postgres>,
    ended: &[(Vec<u8>,)],
) -> Result<(), sqlx::Error> {
    let events: Vec<Event> = ended
        .iter()
        .map(|(session,)| Event::SessionEnded(session.clone()))
        .collect();
    state.instance.tell(tx, &events).await
}

/// Closes this instance's connections of the sessions whose tokens' SHA-256s are `ended`.
pub(crate) fn close_ended(state: &AppState, ended: &[(Vec<u8>,)]) {
    for (session,) in ended {
        state.connections.end_session(session);
    }
}
