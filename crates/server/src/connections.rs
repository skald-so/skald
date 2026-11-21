//! Connections authenticated as a device: `docs/protocol/delivery.md`, "Connecting". A device
//! session gets a challenge; a WebSocket is taken, as its device's, once its first frame signs that
//! challenge with the key of a certified `client` device of the session's account that is not
//! revoked, and an HTTPS request ([`requests`](crate::requests)) once its proof does. A service
//! token gets a challenge too, which only its certified `service` device signs. A taken connection
//! carries the device's sends, its uploads of channels' history and its mailbox, as far as its
//! token's permissions and channels let it, and closes when its session or token ends or its device
//! is revoked.

use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use skald_proto::{AUTH_LABEL, AccountId, Cbor as _, DeviceId, Frame, Push};
use tokio::sync::oneshot;
use tokio::task::AbortHandle;
use tokio::time::Instant;

use crate::delivery::{self, Reader};
use crate::mailboxes::{self, Sender};
use crate::protocol::Stated;
use crate::refusal::Caller;
use crate::service_tokens::{self, Token, live_token};
use crate::sign_in::{bearer, digest, not_signed_in};
use crate::{AppState, database_failed};
use crate::{error_reports, history};

/// How long a challenge works.
pub(crate) const CHALLENGE_LIFETIME: Duration = Duration::from_secs(60);
/// How long the server waits for a connection's `auth`.
const PROOF_WAIT: Duration = Duration::from_secs(5);
/// How long a proved connection waits for the instance to listen to the others.
pub(crate) const LISTEN_WAIT: Duration = Duration::from_secs(5);
/// Why a connection or a request fails when the instance does not listen within [`LISTEN_WAIT`].
pub(crate) const NOT_LISTENING: &str = "the instance does not listen to the others yet";

/// Why the server closes a connection: the reasons of "Connecting".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closing {
    AuthRequired,
    ChallengeRefused,
    DeviceRefused,
    SignatureRefused,
    SessionEnded,
    /// A frame does not decode.
    Malformed,
    /// The database failed.
    ServerError,
}

impl Closing {
    /// The reason it names.
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::AuthRequired => "auth-required",
            Self::ChallengeRefused => "challenge-refused",
            Self::DeviceRefused => "device-refused",
            Self::SignatureRefused => "signature-refused",
            Self::SessionEnded => "session-ended",
            Self::Malformed => "",
            Self::ServerError => "server-error",
        }
    }

    fn frame(self) -> CloseFrame {
        let code = match self {
            Self::Malformed => close_code::INVALID,
            Self::ServerError => close_code::ERROR,
            _ => close_code::POLICY,
        };
        CloseFrame {
            code,
            reason: self.reason().into(),
        }
    }
}

/// Why a proof is not taken.
pub(crate) enum Unproved {
    /// It is refused for this reason.
    Refused(Closing),
    /// The database failed at what it holds, such as `read a challenge`.
    Failed(&'static str, sqlx::Error),
}

impl From<Closing> for Unproved {
    fn from(closing: Closing) -> Self {
        Self::Refused(closing)
    }
}

/// The taken connections of this instance, by their device.
#[derive(Default)]
pub(crate) struct Connections {
    by_device: Mutex<HashMap<DeviceId, Vec<Held>>>,
    next: AtomicU64,
}

/// A taken connection.
struct Held {
    id: u64,
    /// The SHA-256 of its session's token, or of its service token.
    session: Vec<u8>,
    /// Closes it, for the reason it sends.
    end: oneshot::Sender<Closing>,
}

/// A connection of `device` while it is held: dropping it lets the connection go.
pub(crate) struct Registration {
    connections: Arc<Connections>,
    device: DeviceId,
    id: u64,
    /// Closes it at its service token's expiry.
    expiry: Option<AbortHandle>,
}

impl Connections {
    /// Holds a connection of `device` on the session `session`; the receiver fires, with the reason
    /// to close it, when the session ends or the device is revoked.
    fn hold(
        self: &Arc<Self>,
        device: DeviceId,
        session: Vec<u8>,
    ) -> (Registration, oneshot::Receiver<Closing>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (end, ended) = oneshot::channel();
        self.by_device
            .lock()
            .expect("the connections")
            .entry(device)
            .or_default()
            .push(Held { id, session, end });
        let registration = Registration {
            connections: Arc::clone(self),
            device,
            id,
            expiry: None,
        };
        (registration, ended)
    }

    /// The SHA-256s of the tokens of the sessions with connections, each once.
    pub(crate) fn sessions(&self) -> Vec<Vec<u8>> {
        let by_device = self.by_device.lock().expect("the connections");
        let mut sessions: Vec<Vec<u8>> = by_device
            .values()
            .flatten()
            .map(|held| held.session.clone())
            .collect();
        sessions.sort();
        sessions.dedup();
        sessions
    }

    /// Closes every connection of the session whose token's SHA-256 is `session`.
    pub(crate) fn end_session(&self, session: &[u8]) {
        let mut by_device = self.by_device.lock().expect("the connections");
        for held in by_device.values_mut() {
            for ended in held.extract_if(.., |held| held.session == session) {
                // The connection may be closing already.
                let _ = ended.end.send(Closing::SessionEnded);
            }
        }
        by_device.retain(|_, held| !held.is_empty());
    }

    /// Closes the connection `id` of `device`, as at its session's end.
    fn end_one(&self, device: DeviceId, id: u64) {
        let mut by_device = self.by_device.lock().expect("the connections");
        if let Some(held) = by_device.get_mut(&device) {
            for ended in held.extract_if(.., |held| held.id == id) {
                // The connection may be closing already.
                let _ = ended.end.send(Closing::SessionEnded);
            }
            if held.is_empty() {
                by_device.remove(&device);
            }
        }
    }

    /// The devices with connections.
    pub(crate) fn devices(&self) -> Vec<DeviceId> {
        let by_device = self.by_device.lock().expect("the connections");
        by_device.keys().copied().collect()
    }

    /// Closes every connection of `device`, which is revoked.
    pub(crate) fn end_device(&self, device: DeviceId) {
        let held = self
            .by_device
            .lock()
            .expect("the connections")
            .remove(&device);
        for ended in held.into_iter().flatten() {
            // The connection may be closing already.
            let _ = ended.end.send(Closing::DeviceRefused);
        }
    }
}

impl Registration {
    /// Closes the connection at `at`, in seconds since the Unix epoch, as at its session's end.
    fn ends_at(&mut self, at: u64) {
        let left = (UNIX_EPOCH + Duration::from_secs(at))
            .duration_since(SystemTime::now())
            .unwrap_or_default();
        let (connections, device, id) = (Arc::clone(&self.connections), self.device, self.id);
        let ending = tokio::spawn(async move {
            tokio::time::sleep(left).await;
            connections.end_one(device, id);
        });
        self.expiry = Some(ending.abort_handle());
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(expiry) = &self.expiry {
            expiry.abort();
        }
        let mut by_device = self.connections.by_device.lock().expect("the connections");
        if let Some(held) = by_device.get_mut(&self.device) {
            held.retain(|held| held.id != self.id);
            if held.is_empty() {
                by_device.remove(&self.device);
            }
        }
    }
}

/// `Challenge` in the spec.
#[derive(Serialize)]
struct Challenge {
    challenge: String,
}

/// `POST /session/challenge`: a new challenge for the device session or the service token that is
/// the request's bearer token.
pub(crate) async fn challenge(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(Some(token)) = bearer(&headers) else {
        return not_signed_in();
    };
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge).expect("the operating system's random generator failed");
    let issued = sqlx::query(concat!(
        "WITH expired AS (DELETE FROM connection_challenges WHERE expires_at <= now())
         INSERT INTO connection_challenges
             (challenge_hash, session_hash, service_token_hash, expires_at)
         SELECT $1, token_hash, NULL, now() + make_interval(secs => $3) FROM sessions
         WHERE kind = 'device' AND token_hash = $2
         UNION ALL
         SELECT $1, NULL, t.token_hash, now() + make_interval(secs => $3)
         FROM service_tokens t JOIN devices d ON d.id = t.device_id
         WHERE t.token_hash = $2 AND ",
        live_token!(),
    ))
    .bind(Sha256::digest(challenge).to_vec())
    .bind(digest(token))
    .bind(CHALLENGE_LIFETIME.as_secs_f64())
    .execute(&state.db)
    .await;
    match issued {
        Ok(done) if done.rows_affected() > 0 => Json(Challenge {
            challenge: URL_SAFE_NO_PAD.encode(challenge),
        })
        .into_response(),
        Ok(_) => not_signed_in(),
        Err(err) => database_failed("issue a challenge", err),
    }
}

/// `GET /ws`: opens a WebSocket with the subprotocol that states the accepted version. Without a
/// session, as `skald status` opens it, it closes once the proof wait is over.
pub(crate) async fn open(
    State(state): State<AppState>,
    Extension(Stated(version)): Extension<Stated>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade
        .protocols([skald_proto::subprotocol(version)])
        .on_upgrade(move |socket| serve(state, socket, version))
}

/// A connection or a request, once its proof proved its device, held as the device's.
pub(crate) struct Proved {
    pub(crate) device: DeviceId,
    pub(crate) account: AccountId,
    /// The service token of its challenge, for a `service` device.
    pub(crate) token: Option<Token>,
    /// Lets the connection go when dropped.
    pub(crate) _registration: Registration,
    /// Fires when the session ends or the device is revoked.
    pub(crate) ended: oneshot::Receiver<Closing>,
}

/// Takes `socket`, of the protocol version `version`, once it proves its device, and serves it
/// until it closes or its session ends.
async fn serve(state: AppState, mut socket: WebSocket, version: NonZeroU64) {
    match prove(&state, &mut socket).await {
        Ok(Some(proved)) => {
            // Until the instance listens to the others, a connection would miss what they tell.
            if tokio::time::timeout(LISTEN_WAIT, state.instance.listening())
                .await
                .is_err()
            {
                report_closing(&error_reports::logged("serve a connection", &NOT_LISTENING));
                return close(socket, Closing::ServerError).await;
            }
            let sender = Sender {
                device: proved.device,
                account: proved.account,
                version,
                token: proved.token,
            };
            hold(&state, socket, proved.ended, &sender).await;
        }
        Ok(None) => finish(socket).await,
        Err(closing) => close(socket, closing).await,
    }
}

/// Why the server stops serving a connection.
enum Stop {
    /// It closes the connection.
    Close(Closing),
    /// The client is gone.
    Gone,
}

/// Serves `socket`, `sender`'s connection, until the client closes it, `ended` fires, or a frame
/// does not decode: answers its frames in the order they arrive, delivers its device's mailbox and
/// sends the notices of its account's spaces and devices waiting for linking; of its token's space
/// only, for a service token.
async fn hold(
    state: &AppState,
    mut socket: WebSocket,
    mut ended: oneshot::Receiver<Closing>,
    sender: &Sender,
) {
    let mut reader = Reader::new(
        &state.wakers,
        sender.device,
        sender.version,
        state.mailbox_retention,
    );
    let notices = state.notices.listen(sender.account);
    loop {
        let served = tokio::select! {
            // Once the session ends or the device is revoked, nothing more is served.
            biased;
            closing = &mut ended => {
                return close(socket, closing.unwrap_or(Closing::SessionEnded)).await;
            }
            () = reader.woken() => deliver(state, &mut socket, &mut reader, i64::MAX).await,
            noticed = notices.next() => {
                notify(&mut socket, noticed.pushes(sender.token.as_ref())).await
            }
            message = socket.recv() => match message {
                Some(Ok(Message::Binary(bytes))) => {
                    serve_frame(state, &mut socket, sender, &mut reader, &bytes).await
                }
                Some(Ok(Message::Text(_))) => Err(Stop::Close(Closing::Malformed)),
                Some(Ok(Message::Close(_))) => return finish(socket).await,
                Some(Err(_)) | None => return,
                Some(Ok(_)) => Ok(()),
            },
        };
        match served {
            Ok(()) => {}
            Err(Stop::Close(closing)) => return close(socket, closing).await,
            Err(Stop::Gone) => return,
        }
    }
}

/// Serves the frame `bytes` from `sender`: answers a `send` or a `history`, acks a `sync` or an
/// `ack`, and ignores a frame of another type. A service token without the receive permission
/// gets nothing of the mailbox: its `sync` and `ack` are ignored too.
async fn serve_frame(
    state: &AppState,
    socket: &mut WebSocket,
    sender: &Sender,
    reader: &mut Reader,
    bytes: &[u8],
) -> Result<(), Stop> {
    let answered = match Frame::from_cbor(bytes) {
        Ok(Frame::Send { envelope, report }) => {
            mailboxes::send(
                &state.db,
                &state.wakers,
                state.limits,
                sender,
                &envelope,
                report,
            )
            .await
        }
        Ok(Frame::History {
            upload,
            space,
            channel,
            item,
        }) => {
            let answer = history::upload(&state.db, sender, upload, space, channel, &item)
                .await
                .map_err(|err| Stop::Close(failed("keep a history upload", err)))?;
            return send_frame(socket, &answer).await;
        }
        Ok(Frame::Sync { .. } | Frame::Ack { .. }) if !receives(sender.token.as_ref()) => {
            return Ok(());
        }
        Ok(Frame::Sync { cursor }) => return sync(state, socket, reader, cursor).await,
        Ok(Frame::Ack { seq }) => return ack(state, sender.device, seq.get()).await,
        // A frame of a type the server does not know, or one it does not serve yet.
        Ok(_) => return Ok(()),
        Err(_) => match Frame::misaddressed_send(bytes) {
            Some(header) => {
                mailboxes::misaddressed(&state.db, &state.wakers, sender, &header).await
            }
            None => return Err(Stop::Close(Closing::Malformed)),
        },
    };
    let answer = answered.map_err(|err| Stop::Close(failed("store a send", err)))?;
    send_frame(socket, &answer).await
}

/// Acks `reader`'s mailbox up to `cursor`; at the connection's first `sync`, sends the entries
/// after `cursor` that the mailbox holds, then an empty `batch`.
async fn sync(
    state: &AppState,
    socket: &mut WebSocket,
    reader: &mut Reader,
    cursor: u64,
) -> Result<(), Stop> {
    ack(state, reader.device(), cursor).await?;
    let held = reader
        .start(&state.db)
        .await
        .map_err(|err| Stop::Close(failed("read a mailbox", err)))?;
    let Some(held) = held else {
        return Ok(());
    };
    deliver(state, socket, reader, held).await?;
    send_frame(socket, &Frame::Batch(Vec::new())).await
}

/// Acks `device`'s mailbox up to `cursor`.
async fn ack(state: &AppState, device: DeviceId, cursor: u64) -> Result<(), Stop> {
    delivery::ack(&state.db, &state.wakers, device, cursor)
        .await
        .map_err(|err| Stop::Close(failed("ack a mailbox", err)))
}

/// Sends the entries of `reader`'s mailbox after those sent, up to `seq` `upto`.
async fn deliver(
    state: &AppState,
    socket: &mut WebSocket,
    reader: &mut Reader,
    upto: i64,
) -> Result<(), Stop> {
    while let Some(page) = reader
        .next(&state.db, upto)
        .await
        .map_err(|err| Stop::Close(failed("read a mailbox", err)))?
    {
        for push in page {
            send_frame(socket, &Frame::Push(push)).await?;
        }
    }
    Ok(())
}

/// Sends each of the notices `pushes` on `socket`.
async fn notify(socket: &mut WebSocket, pushes: Vec<Push>) -> Result<(), Stop> {
    for push in pushes {
        send_frame(socket, &Frame::Push(push)).await?;
    }
    Ok(())
}

/// Sends `frame` on `socket`.
async fn send_frame(socket: &mut WebSocket, frame: &Frame) -> Result<(), Stop> {
    socket
        .send(Message::Binary(frame.to_cbor().into()))
        .await
        .map_err(|_| Stop::Gone)
}

/// Reads `socket`'s `auth`, checks it and holds the connection as its device's. `None` if the
/// client closed the connection first.
async fn prove(state: &AppState, socket: &mut WebSocket) -> Result<Option<Proved>, Closing> {
    let Some(frame) = first_frame(socket).await? else {
        return Ok(None);
    };
    let Ok(Frame::Auth {
        challenge,
        device,
        signature,
    }) = Frame::from_cbor(&frame)
    else {
        return Err(Closing::AuthRequired);
    };
    let proof = Proof {
        challenge: &challenge,
        device,
        signature: &signature,
        label: AUTH_LABEL,
    };
    match check(state, &proof, None).await {
        Ok(proved) => Ok(Some(proved)),
        Err(Unproved::Refused(closing)) => Err(closing),
        Err(Unproved::Failed(doing, err)) => Err(failed(doing, err)),
    }
}

/// A proof of a device: by its signing key, with `label`, over `challenge` and the origin of the
/// server's public address.
pub(crate) struct Proof<'p> {
    pub(crate) challenge: &'p [u8],
    pub(crate) device: DeviceId,
    pub(crate) signature: &'p [u8; 64],
    pub(crate) label: &'static str,
}

/// Checks `proof`, of a challenge of the session or service token whose token's SHA-256 is
/// `session` if given, and holds a connection as its device's, until its token's expiry for a
/// service token. The challenge is used up, whether the proof is taken or refused.
pub(crate) async fn check(
    state: &AppState,
    proof: &Proof<'_>,
    session: Option<&[u8]>,
) -> Result<Proved, Unproved> {
    let of = challenge_session(state, proof.challenge)
        .await?
        .ok_or(Closing::ChallengeRefused)?;
    // Held before the challenge is used, so that once the use finds the session or token, its end
    // closes the connection; an end before the use deletes the challenge.
    let (mut registration, ended) = state.connections.hold(proof.device, of.clone());
    let bearer = take_challenge(state, proof.challenge)
        .await?
        .filter(|_| session.is_none_or(|session| session == of))
        .ok_or(Closing::ChallengeRefused)?;
    let signing = device_key(state, proof.device, &bearer)
        .await?
        .ok_or(Closing::DeviceRefused)?;
    let origin = state
        .sign_in
        .as_ref()
        .map(|sign_in| sign_in.public_url.origin())
        .ok_or(Closing::SignatureRefused)?;
    skald_crypto::verify(
        &signing,
        proof.label,
        &[proof.challenge, origin.as_bytes()],
        proof.signature,
    )
    .map_err(|_| Closing::SignatureRefused)?;
    let (account, token) = match bearer {
        Caller::Session(account) => {
            // The session lists the device it proved, and its use to the minute, as
            // `sign_in::find` does.
            sqlx::query(
                "UPDATE sessions SET device_id = $1::uuid, last_used_at = now()
                 WHERE token_hash = $2 AND (device_id IS DISTINCT FROM $1::uuid
                     OR last_used_at <= now() - interval '1 minute')",
            )
            .bind(proof.device.to_string())
            .bind(&of)
            .execute(&state.db)
            .await
            .map_err(|err| Unproved::Failed("record a session's device", err))?;
            (account.parse().expect("a stored account ID"), None)
        }
        Caller::Token(token) => {
            registration.ends_at(token.expires);
            (token.account, Some(token))
        }
    };
    Ok(Proved {
        device: proof.device,
        account,
        token,
        _registration: registration,
        ended,
    })
}

/// Whether a connection of `token`, if it has one, reads and acks its device's mailbox.
pub(crate) fn receives(token: Option<&Token>) -> bool {
    token.is_none_or(Token::receives)
}

/// The bytes of `socket`'s first frame, a binary message, within [`PROOF_WAIT`]; `None` if the
/// client closed the connection first.
async fn first_frame(socket: &mut WebSocket) -> Result<Option<Vec<u8>>, Closing> {
    let deadline = Instant::now() + PROOF_WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, socket.recv())
            .await
            .map_err(|_| Closing::AuthRequired)?;
        match message {
            Some(Ok(Message::Binary(bytes))) => return Ok(Some(bytes.to_vec())),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(Message::Text(_))) => return Err(Closing::AuthRequired),
            Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(None),
        }
    }
}

/// The SHA-256 of the token of `challenge`'s session or service token, if the server holds the
/// challenge.
async fn challenge_session(
    state: &AppState,
    challenge: &[u8],
) -> Result<Option<Vec<u8>>, Unproved> {
    sqlx::query_as(
        "SELECT coalesce(session_hash, service_token_hash) FROM connection_challenges
         WHERE challenge_hash = $1",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .fetch_optional(&state.db)
    .await
    .map(|found| found.map(|(session,)| session))
    .map_err(|err| Unproved::Failed("read a challenge", err))
}

/// Uses up `challenge`: its session's account or its service token, if it is one the server
/// issued, unused, unexpired, to a session or token that has not ended.
async fn take_challenge(state: &AppState, challenge: &[u8]) -> Result<Option<Caller>, Unproved> {
    let taken: Option<(Option<String>, Option<Vec<u8>>, bool)> = sqlx::query_as(
        "WITH c AS (
             DELETE FROM connection_challenges WHERE challenge_hash = $1
             RETURNING session_hash, service_token_hash, expires_at > now() AS unexpired
         )
         SELECT s.account_id::text, c.service_token_hash, c.unexpired
         FROM c LEFT JOIN sessions s ON s.token_hash = c.session_hash",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Unproved::Failed("take a challenge", err))?;
    match taken {
        Some((Some(account), _, true)) => Ok(Some(Caller::Session(account))),
        Some((None, Some(token), true)) => service_tokens::live(&state.db, &token)
            .await
            .map(|token| token.map(Caller::Token))
            .map_err(|err| Unproved::Failed("read a service token", err)),
        _ => Ok(None),
    }
}

/// The signing key of `device` if it has a certificate on the server and is not revoked, and is a
/// `client` device of a session's account, or a service token's `service` device.
async fn device_key(
    state: &AppState,
    device: DeviceId,
    bearer: &Caller,
) -> Result<Option<[u8; 32]>, Unproved> {
    let (account, kind) = match bearer {
        Caller::Session(account) => (account.clone(), "client"),
        Caller::Token(token) if token.device == device => (token.account.to_string(), "service"),
        Caller::Token(_) => return Ok(None),
    };
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT signing FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid AND kind = $3 AND cert IS NOT NULL
           AND NOT revoked",
    )
    .bind(device.to_string())
    .bind(account)
    .bind(kind)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| Unproved::Failed("read a device", err))?;
    Ok(found.and_then(|(signing,)| signing.try_into().ok()))
}

/// The closing when the database fails at `doing`; the failure goes to the server's output and its
/// error reports.
fn failed(doing: &str, err: sqlx::Error) -> Closing {
    let cause = error_reports::logged(doing, &err);
    report_closing(&cause);
    Closing::ServerError
}

/// Reports that a connection closes with `server-error` for `cause`.
fn report_closing(cause: &str) {
    error_reports::report(
        &format!("a connection closed with server-error: {cause}"),
        Some("GET /ws"),
    );
}

/// Answers the client's close of `socket`: the next read sends the closing handshake's answer.
async fn finish(mut socket: WebSocket) {
    while let Some(Ok(_)) = socket.recv().await {}
}

/// Closes `socket` for `closing`.
async fn close(mut socket: WebSocket, closing: Closing) {
    // The client may be gone already.
    let _ = socket.send(Message::Close(Some(closing.frame()))).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(byte: u8) -> DeviceId {
        DeviceId::from_signing_key(&[byte; 32])
    }

    /// The number of held connections of each device.
    fn held(connections: &Connections) -> HashMap<DeviceId, usize> {
        let by_device = connections.by_device.lock().unwrap();
        by_device
            .iter()
            .map(|(device, held)| (*device, held.len()))
            .collect()
    }

    #[test]
    fn a_connection_is_let_go_when_its_registration_drops() {
        let connections = Arc::new(Connections::default());
        let (first, _) = connections.hold(device(1), vec![1]);
        let (second, mut ended) = connections.hold(device(1), vec![2]);
        assert_eq!(held(&connections), HashMap::from([(device(1), 2)]));

        drop(first);
        assert_eq!(held(&connections), HashMap::from([(device(1), 1)]));
        // The other connection is still held.
        connections.end_session(&[2]);
        assert_eq!(ended.try_recv(), Ok(Closing::SessionEnded));
        drop(second);
        assert_eq!(held(&connections), HashMap::new());
    }

    /// Held connections of device 1 on sessions 1 and 2, and of device 2 on session 1.
    fn three(connections: &Arc<Connections>) -> [(Registration, oneshot::Receiver<Closing>); 3] {
        [
            connections.hold(device(1), vec![1]),
            connections.hold(device(1), vec![2]),
            connections.hold(device(2), vec![1]),
        ]
    }

    #[test]
    fn ending_a_session_closes_its_connections_only() {
        let connections = Arc::new(Connections::default());
        let [(_a, mut a), (_b, mut b), (_c, mut c)] = three(&connections);

        connections.end_session(&[1]);

        assert_eq!(a.try_recv(), Ok(Closing::SessionEnded));
        assert_eq!(c.try_recv(), Ok(Closing::SessionEnded));
        assert_eq!(b.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        assert_eq!(held(&connections), HashMap::from([(device(1), 1)]));
    }

    #[test]
    fn revoking_a_device_closes_its_connections_only() {
        let connections = Arc::new(Connections::default());
        let [(_a, mut a), (_b, mut b), (_c, mut c)] = three(&connections);
        assert_eq!(
            sorted(connections.devices()),
            sorted(vec![device(1), device(2)])
        );

        connections.end_device(device(1));

        assert_eq!(a.try_recv(), Ok(Closing::DeviceRefused));
        assert_eq!(b.try_recv(), Ok(Closing::DeviceRefused));
        assert_eq!(c.try_recv(), Err(oneshot::error::TryRecvError::Empty));
        assert_eq!(held(&connections), HashMap::from([(device(2), 1)]));
        assert_eq!(connections.devices(), [device(2)]);
    }

    fn sorted(mut devices: Vec<DeviceId>) -> Vec<DeviceId> {
        devices.sort();
        devices
    }
}
