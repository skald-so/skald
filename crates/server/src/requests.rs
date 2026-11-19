//! A device's mailbox over HTTPS requests: `docs/protocol/delivery.md`, "Frames", "Connecting"
//! and "Mailbox". Each request carries a proof of its device, as a WebSocket's `auth` does, on a
//! challenge of its bearer token's session or service token; the server reads, acks and stores
//! nothing for a request it does not prove, and a service token reads and acks only with the
//! receive permission. A `send` gets what it gets on a WebSocket, an `ack` acks, and a long-poll
//! acks its cursor and answers a `batch` of the entries after it and the notices since the device's
//! previous long-poll answer, at once or with the first to come within its wait.

use std::time::Duration;

use axum::Extension;
use axum::body::{Body, Bytes};
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use skald_proto::{Cbor as _, DeviceId, Frame, PROOF_HEADER, Push, REQUEST_LABEL, RequestProof};
use sqlx::PgPool;

use crate::connections::{self, Closing, LISTEN_WAIT, NOT_LISTENING, Proof, Proved, Unproved};
use crate::delivery::{self, Reader};
use crate::mailboxes::{self, Sender};
use crate::protocol::Stated;
use crate::sign_in::{bearer, digest, not_signed_in, param};
use crate::{AppState, database_failed, error_reports, notices, problem, service_tokens};

/// The longest wait of a long-poll without the server setting.
pub const LONG_POLL_MAX_WAIT: Duration = Duration::from_secs(30);
/// The longest body of a request: the longest message a WebSocket takes.
const BODY_LIMIT: usize = 64 << 20;
/// The media type of frames in bodies.
const CBOR: &str = "application/cbor";

/// `POST /mailbox/send`: stores the envelope of the `send` frame in the body; answers its `stored`
/// or `send-refused`.
pub(crate) async fn send(
    State(state): State<AppState>,
    Extension(Stated(version)): Extension<Stated>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (proved, body) = match proved_with(&state, &headers, body).await {
        Ok(proved) => proved,
        Err(refused) => return *refused,
    };
    let sender = Sender {
        device: proved.device,
        account: proved.account,
        version,
        token: proved.token,
    };
    let (db, wakers) = (&state.db, &state.wakers);
    let answered = match Frame::from_cbor(&body) {
        Ok(Frame::Send { envelope, report }) => {
            mailboxes::send(db, wakers, state.limits, &sender, &envelope, report).await
        }
        Ok(_) => return invalid("The body is not a send frame."),
        Err(_) => match Frame::misaddressed_send(&body) {
            Some(header) => mailboxes::misaddressed(db, wakers, &sender, &header).await,
            None => return invalid("The body is not a send frame."),
        },
    };
    match answered {
        Ok(answer) => frame(&answer),
        Err(err) => database_failed("store a send", err),
    }
}

/// `POST /mailbox/ack`: acks the device's mailbox up to the `seq` of the `ack` frame in the body.
pub(crate) async fn ack(State(state): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let (proved, body) = match receiving_with(&state, &headers, body).await {
        Ok(proved) => proved,
        Err(refused) => return *refused,
    };
    let Ok(Frame::Ack { seq }) = Frame::from_cbor(&body) else {
        return invalid("The body is not an ack frame.");
    };
    match delivery::ack(&state.db, &state.wakers, proved.device, seq.get()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => database_failed("ack a mailbox", err),
    }
}

/// `POST /mailbox/poll?wait=<seconds>`: acks the device's mailbox up to the cursor of the `sync`
/// frame in the body, and answers a `batch` of the entries after it and the notices since the
/// device's previous long-poll answer: at once when there are some or the wait is 0, otherwise with
/// the first to come within the wait, up to the server's longest, or empty at its end. Ends with
/// the refusal a new request would get when its session ends or its device is revoked.
pub(crate) async fn poll(
    State(state): State<AppState>,
    Extension(Stated(version)): Extension<Stated>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (mut proved, body) = match receiving_with(&state, &headers, body).await {
        Ok(proved) => proved,
        Err(refused) => return *refused,
    };
    let Some(wait) = param(query.as_deref(), "wait").and_then(|wait| wait.parse::<u64>().ok())
    else {
        return invalid("The query has no wait in seconds.");
    };
    let Ok(Frame::Sync { cursor }) = Frame::from_cbor(&body) else {
        return invalid("The body is not a sync frame.");
    };
    let wait = Duration::from_secs(wait).min(state.long_poll_max_wait);
    // Listened to before reading, so that nothing added meanwhile is missed.
    let mut reader = Reader::new(
        &state.wakers,
        proved.device,
        version,
        state.mailbox_retention,
    )
    .after(cursor);
    let notices = state.notices.listen(proved.account);
    // Only a long-poll that waits needs what other instances tell.
    if let Err(failed) = ack_listening(&state, proved.device, cursor, !wait.is_zero()).await {
        return *failed;
    }
    let ends = tokio::time::sleep(wait);
    tokio::pin!(ends);
    loop {
        let pushes = match answer(&state.db, &mut reader, &proved).await {
            Ok(pushes) => pushes,
            Err((doing, err)) => return database_failed(doing, err),
        };
        if !pushes.is_empty() || wait.is_zero() {
            return frame(&Frame::Batch(pushes));
        }
        tokio::select! {
            // Once the session ends or the device is revoked, nothing more is answered.
            biased;
            closing = &mut proved.ended => {
                return refusal(closing.unwrap_or(Closing::SessionEnded));
            }
            () = reader.woken() => {}
            _ = notices.next() => {}
            () = &mut ends => return frame(&Frame::Batch(Vec::new())),
        }
    }
}

/// Acks `device`'s mailbox up to `cursor`, once the instance listens to the others if `listen`, so
/// that a long-poll waiting on its mailbox misses nothing they tell; the answer if either fails.
pub(crate) async fn ack_listening(
    state: &AppState,
    device: DeviceId,
    cursor: u64,
    listen: bool,
) -> Result<(), Box<Response>> {
    if listen {
        listening(state).await?;
    }
    delivery::ack(&state.db, &state.wakers, device, cursor)
        .await
        .map_err(|err| Box::new(database_failed("ack a mailbox", err)))
}

/// Waits until the instance listens to the others; the answer if it does not in time, which also
/// goes to the server's output and its error reports.
pub(crate) async fn listening(state: &AppState) -> Result<(), Box<Response>> {
    match tokio::time::timeout(LISTEN_WAIT, state.instance.listening()).await {
        Ok(()) => Ok(()),
        Err(_) => {
            let cause = error_reports::logged("answer a request", &NOT_LISTENING);
            Err(Box::new(error_reports::with_cause(
                server_error("The server does not listen to its other instances yet."),
                cause,
            )))
        }
    }
}

/// What a long-poll of `proved`'s device answers now: the next entries of `reader`, then a
/// `space-notice` of each space with a revision since its previous answer.
async fn answer(
    db: &PgPool,
    reader: &mut Reader,
    proved: &Proved,
) -> Result<Vec<Push>, (&'static str, sqlx::Error)> {
    let mut pushes = Vec::new();
    while let Some(page) = reader
        .next(db, i64::MAX)
        .await
        .map_err(|err| ("read a mailbox", err))?
    {
        if !page.is_empty() {
            pushes = page;
            break;
        }
    }
    let spaces = notices::since_last_answer(db, proved.device, proved.account)
        .await
        .map_err(|err| ("read the notices", err))?;
    let spaces = service_tokens::seen(proved.token.as_ref(), spaces);
    pushes.extend(spaces.into_iter().map(|space| Push::SpaceNotice { space }));
    Ok(pushes)
}

/// Proves the request of `headers` and reads its `body`; the answer to the request if either
/// fails.
async fn proved_with(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<(Proved, Bytes), Box<Response>> {
    let proved = prove(state, headers).await.map_err(Box::new)?;
    let body = axum::body::to_bytes(body, BODY_LIMIT).await.map_err(|_| {
        Box::new(problem(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too-large",
            "Request too large",
            "The body is over 64 MiB.",
        ))
    })?;
    Ok((proved, body))
}

/// The request of `headers` proved, as `proved_with` does, and allowed to read and ack its
/// device's mailbox, with its body; the refusal otherwise.
async fn receiving_with(
    state: &AppState,
    headers: &HeaderMap,
    body: Body,
) -> Result<(Proved, Bytes), Box<Response>> {
    let (proved, body) = proved_with(state, headers, body).await?;
    receiving(&proved).map_err(Box::new)?;
    Ok((proved, body))
}

/// Checks the proof of the request of `headers`, on a challenge of its bearer token's session or
/// service token, and holds it as its device's; the refusal if the server does not take it.
#[expect(
    clippy::result_large_err,
    reason = "the refusal is the request's answer"
)]
pub(crate) async fn prove(state: &AppState, headers: &HeaderMap) -> Result<Proved, Response> {
    let Some(Some(token)) = bearer(headers) else {
        return Err(not_signed_in());
    };
    let session = digest(token);
    match bearer_lives(&state.db, &session).await {
        Ok(true) => {}
        Ok(false) => return Err(not_signed_in()),
        Err(err) => return Err(database_failed("read a session", err)),
    }
    let Some(proof) = proof_of(headers) else {
        return Err(refusal(Closing::AuthRequired));
    };
    let checked = Proof {
        challenge: &proof.challenge,
        device: proof.device,
        signature: &proof.signature,
        label: REQUEST_LABEL,
    };
    connections::check(state, &checked, Some(&session))
        .await
        .map_err(|unproved| match unproved {
            Unproved::Refused(closing) => refusal(closing),
            Unproved::Failed(doing, err) => database_failed(doing, err),
        })
}

/// Whether the token of a device session, or a service token that has not ended, has the SHA-256
/// `session`.
async fn bearer_lives(db: &PgPool, session: &[u8]) -> Result<bool, sqlx::Error> {
    let found: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM sessions WHERE kind = 'device' AND token_hash = $1
         UNION ALL
         SELECT 1 FROM service_tokens t JOIN devices d ON d.id = t.device_id
         WHERE t.token_hash = $1 AND t.expires_at > now() AND NOT d.revoked",
    )
    .bind(session)
    .fetch_optional(db)
    .await?;
    Ok(found.is_some())
}

/// Refuses `proved` unless it reads and acks its device's mailbox: a service token's needs the
/// receive permission.
#[expect(
    clippy::result_large_err,
    reason = "the refusal is the request's answer"
)]
pub(crate) fn receiving(proved: &Proved) -> Result<(), Response> {
    if connections::receives(proved.token.as_ref()) {
        return Ok(());
    }
    Err(problem(
        StatusCode::FORBIDDEN,
        "not-allowed",
        "Not allowed",
        "The service token has no receive permission.",
    ))
}

/// The proof in the request's only `Skald-Device-Proof` header, if it decodes.
fn proof_of(headers: &HeaderMap) -> Option<RequestProof> {
    let mut values = headers.get_all(PROOF_HEADER).iter();
    let (Some(value), None) = (values.next(), values.next()) else {
        return None;
    };
    let bytes = URL_SAFE_NO_PAD.decode(value.as_bytes()).ok()?;
    RequestProof::from_cbor(&bytes).ok()
}

/// The answer to a request the server does not prove, for `closing`: what a new request gets.
fn refusal(closing: Closing) -> Response {
    let unauthorized = StatusCode::UNAUTHORIZED;
    match closing {
        Closing::SessionEnded => not_signed_in(),
        Closing::AuthRequired => problem(
            unauthorized,
            closing.reason(),
            "Device proof required",
            "The request has no valid Skald-Device-Proof header.",
        ),
        Closing::ChallengeRefused => problem(
            unauthorized,
            closing.reason(),
            "Challenge refused",
            "The proof's challenge is unknown, used, expired or of another session.",
        ),
        Closing::DeviceRefused => problem(
            unauthorized,
            closing.reason(),
            "Device refused",
            "The proof's device is not a certified client device of the session's account, or is revoked.",
        ),
        Closing::SignatureRefused => problem(
            unauthorized,
            closing.reason(),
            "Signature refused",
            "The proof's signature does not verify over the server's public address.",
        ),
        Closing::Malformed => invalid("The request is malformed."),
        Closing::ServerError => server_error("The server failed to check the proof."),
    }
}

/// The answer to an invalid request, for the reason `detail`.
pub(crate) fn invalid(detail: &str) -> Response {
    problem(
        StatusCode::BAD_REQUEST,
        "invalid-request",
        "Invalid request",
        detail,
    )
}

/// The answer when the server fails, for the reason `detail`.
fn server_error(detail: &str) -> Response {
    problem(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server-error",
        "The server failed",
        detail,
    )
}

/// The answer that carries `frame`.
fn frame(frame: &Frame) -> Response {
    ([(header::CONTENT_TYPE, CBOR)], frame.to_cbor()).into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use skald_proto::DeviceId;

    use super::*;

    fn proof() -> RequestProof {
        RequestProof {
            challenge: vec![1; 32],
            device: DeviceId::from_signing_key(&[2; 32]),
            signature: [3; 64],
        }
    }

    fn headers(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append(PROOF_HEADER, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn the_proof_is_the_only_header_in_base64url_without_padding() {
        let encoded = URL_SAFE_NO_PAD.encode(proof().to_cbor());
        assert_eq!(proof_of(&headers(&[&encoded])), Some(proof()));

        let padded = base64::engine::general_purpose::URL_SAFE.encode(proof().to_cbor());
        let standard = base64::engine::general_purpose::STANDARD.encode([0xfb; 99]);
        let not_cbor = URL_SAFE_NO_PAD.encode([0xff]);
        for values in [
            vec![],
            vec![encoded.as_str(), encoded.as_str()],
            vec![padded.as_str()],
            vec![standard.as_str()],
            vec![not_cbor.as_str()],
        ] {
            assert_eq!(proof_of(&headers(&values)), None, "{values:?}");
        }
    }

    #[tokio::test]
    async fn a_refused_request_gets_the_reason_of_a_websocket_or_not_signed_in() {
        let reasons = [
            (Closing::AuthRequired, 401, "auth-required"),
            (Closing::ChallengeRefused, 401, "challenge-refused"),
            (Closing::DeviceRefused, 401, "device-refused"),
            (Closing::SignatureRefused, 401, "signature-refused"),
            (Closing::SessionEnded, 401, "not-signed-in"),
            (Closing::Malformed, 400, "invalid-request"),
            (Closing::ServerError, 500, "server-error"),
        ];
        for (closing, status, kind) in reasons {
            let response = refusal(closing);
            assert_eq!(response.status(), status, "{closing:?}");
            let body = axum::body::to_bytes(response.into_body(), 4_096).await;
            let body: serde_json::Value = serde_json::from_slice(&body.unwrap()).unwrap();
            let expected = format!("https://skald.so/problems/{kind}");
            assert_eq!(body["type"], expected.as_str(), "{closing:?}");
        }
    }
}
