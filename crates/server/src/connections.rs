//! Connections authenticated as a device: `docs/protocol/delivery.md`, "Connecting". A device
//! session gets a challenge; a WebSocket is taken, as its device's, once its first frame signs that
//! challenge with the key of a certified `client` device of the session's account. A taken
//! connection closes when its session ends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade, close_code};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use sha2::{Digest as _, Sha256};
use skald_proto::{AUTH_LABEL, Cbor as _, DeviceId, Frame};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::protocol::Stated;
use crate::sign_in::{bearer, digest, not_signed_in};
use crate::{AppState, database_failed};

/// How long a challenge works.
const CHALLENGE_LIFETIME: Duration = Duration::from_secs(60);
/// How long the server waits for a connection's `auth`.
const PROOF_WAIT: Duration = Duration::from_secs(5);

/// Why the server closes a connection: the reasons of "Connecting".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    AuthRequired,
    ChallengeRefused,
    DeviceRefused,
    SignatureRefused,
    SessionEnded,
    /// The database failed.
    ServerError,
}

impl Closing {
    fn frame(self) -> CloseFrame {
        let (code, reason) = match self {
            Self::AuthRequired => (close_code::POLICY, "auth-required"),
            Self::ChallengeRefused => (close_code::POLICY, "challenge-refused"),
            Self::DeviceRefused => (close_code::POLICY, "device-refused"),
            Self::SignatureRefused => (close_code::POLICY, "signature-refused"),
            Self::SessionEnded => (close_code::POLICY, "session-ended"),
            Self::ServerError => (close_code::ERROR, "server-error"),
        };
        CloseFrame {
            code,
            reason: reason.into(),
        }
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
    /// The SHA-256 of its session's token.
    session: Vec<u8>,
    /// Closes it.
    end: oneshot::Sender<()>,
}

/// A connection of `device` while it is held: dropping it lets the connection go.
struct Registration {
    connections: Arc<Connections>,
    device: DeviceId,
    id: u64,
}

impl Connections {
    /// Holds a connection of `device` on the session `session`; the receiver fires when the session
    /// ends.
    fn hold(
        self: &Arc<Self>,
        device: DeviceId,
        session: Vec<u8>,
    ) -> (Registration, oneshot::Receiver<()>) {
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
        };
        (registration, ended)
    }

    /// Closes every connection of the session whose token's SHA-256 is `session`.
    pub(crate) fn end_session(&self, session: &[u8]) {
        let mut by_device = self.by_device.lock().expect("the connections");
        for held in by_device.values_mut() {
            for ended in held.extract_if(.., |held| held.session == session) {
                // The connection may be closing already.
                let _ = ended.end.send(());
            }
        }
        by_device.retain(|_, held| !held.is_empty());
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
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

/// `POST /session/challenge`: a new challenge for the device session of the request's bearer
/// token.
pub(crate) async fn challenge(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(Some(token)) = bearer(&headers) else {
        return not_signed_in();
    };
    let mut challenge = [0; 32];
    getrandom::fill(&mut challenge).expect("the operating system's random generator failed");
    let issued = sqlx::query(
        "WITH expired AS (DELETE FROM connection_challenges WHERE expires_at <= now())
         INSERT INTO connection_challenges (challenge_hash, session_hash, expires_at)
         SELECT $1, token_hash, now() + make_interval(secs => $3) FROM sessions
         WHERE kind = 'device' AND token_hash = $2",
    )
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
        .on_upgrade(move |socket| serve(state, socket))
}

/// A connection's device and session, once its `auth` proved them.
struct Proved {
    device: DeviceId,
    /// The SHA-256 of the session's token.
    session: Vec<u8>,
}

/// Takes `socket` once it proves its device, and holds it until it closes or its session ends.
async fn serve(state: AppState, mut socket: WebSocket) {
    let proved = match prove(&state, &mut socket).await {
        Ok(Some(proved)) => proved,
        // The client closed it.
        Ok(None) => return,
        Err(closing) => return close(socket, closing).await,
    };
    let (_registration, ended) = state
        .connections
        .hold(proved.device, proved.session.clone());
    // The session may have ended between the proof and the hold.
    match session_exists(&state, &proved.session).await {
        Ok(true) => {}
        Ok(false) => return close(socket, Closing::SessionEnded).await,
        Err(closing) => return close(socket, closing).await,
    }
    hold(socket, ended).await;
}

/// Keeps `socket` open until the client closes it or `ended` fires. It carries no frame after
/// `auth` yet.
async fn hold(mut socket: WebSocket, mut ended: oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut ended => return close(socket, Closing::SessionEnded).await,
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_)) | Err(_)) | None => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Reads `socket`'s `auth` and checks it; `None` if the client closed the connection first.
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
    let (session, account) = take_challenge(state, &challenge)
        .await?
        .ok_or(Closing::ChallengeRefused)?;
    let signing = client_key(state, device, &account)
        .await?
        .ok_or(Closing::DeviceRefused)?;
    let origin = state
        .sign_in
        .as_ref()
        .map(|sign_in| sign_in.public_url.origin())
        .ok_or(Closing::SignatureRefused)?;
    skald_crypto::verify(
        &signing,
        AUTH_LABEL,
        &[&challenge, origin.as_bytes()],
        &signature,
    )
    .map_err(|_| Closing::SignatureRefused)?;
    Ok(Some(Proved { device, session }))
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

/// Uses up `challenge`: the SHA-256 of its session's token and the session's account, if it is
/// one the server issued, unused, unexpired, to a session that has not ended.
async fn take_challenge(
    state: &AppState,
    challenge: &[u8],
) -> Result<Option<(Vec<u8>, String)>, Closing> {
    let taken: Option<(Vec<u8>, String, bool)> = sqlx::query_as(
        "DELETE FROM connection_challenges c USING sessions s
         WHERE c.challenge_hash = $1 AND s.token_hash = c.session_hash
         RETURNING c.session_hash, s.account_id::text, c.expires_at > now()",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| failed("take a challenge", err))?;
    Ok(taken
        .filter(|(_, _, unexpired)| *unexpired)
        .map(|(session, account, _)| (session, account)))
}

/// The signing key of `device` if it is a `client` device of `account` with a certificate on the
/// server.
async fn client_key(
    state: &AppState,
    device: DeviceId,
    account: &str,
) -> Result<Option<[u8; 32]>, Closing> {
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT signing FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid AND kind = 'client' AND cert IS NOT NULL",
    )
    .bind(device.to_string())
    .bind(account)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| failed("read a device", err))?;
    Ok(found.and_then(|(signing,)| signing.try_into().ok()))
}

/// Whether the session whose token's SHA-256 is `session` has not ended.
async fn session_exists(state: &AppState, session: &[u8]) -> Result<bool, Closing> {
    sqlx::query("SELECT 1 FROM sessions WHERE token_hash = $1")
        .bind(session)
        .fetch_optional(&state.db)
        .await
        .map(|found| found.is_some())
        .map_err(|err| failed("read a session", err))
}

/// The closing when the database fails at `doing`; the failure goes to the server's output.
fn failed(doing: &str, err: sqlx::Error) -> Closing {
    eprintln!("skald-server: cannot {doing}: {err}");
    Closing::ServerError
}

/// Closes `socket` for `closing`.
async fn close(mut socket: WebSocket, closing: Closing) {
    // The client may be gone already.
    let _ = socket.send(Message::Close(Some(closing.frame()))).await;
}
