//! Connections authenticated as a device: `docs/protocol/delivery.md`, "Connecting". A device
//! session gets a challenge; a WebSocket is taken, as its device's, once its first frame signs that
//! challenge with the key of a certified `client` device of the session's account that is not
//! revoked. A taken connection carries the device's sends and its mailbox, and closes when its
//! session ends or its device is revoked.

use std::collections::HashMap;
use std::num::NonZeroU64;
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
use skald_proto::{AUTH_LABEL, AccountId, Cbor as _, DeviceId, Frame, Push, SpaceId};
use tokio::sync::oneshot;
use tokio::time::Instant;

use crate::delivery::{self, Reader};
use crate::mailboxes::{self, Sender};
use crate::protocol::Stated;
use crate::sign_in::{bearer, digest, not_signed_in};
use crate::{AppState, database_failed};

/// How long a challenge works.
const CHALLENGE_LIFETIME: Duration = Duration::from_secs(60);
/// How long the server waits for a connection's `auth`.
const PROOF_WAIT: Duration = Duration::from_secs(5);
/// How long a proved connection waits for the instance to listen to the others.
const LISTEN_WAIT: Duration = Duration::from_secs(5);

/// Why the server closes a connection: the reasons of "Connecting".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
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
    fn frame(self) -> CloseFrame {
        let (code, reason) = match self {
            Self::AuthRequired => (close_code::POLICY, "auth-required"),
            Self::ChallengeRefused => (close_code::POLICY, "challenge-refused"),
            Self::DeviceRefused => (close_code::POLICY, "device-refused"),
            Self::SignatureRefused => (close_code::POLICY, "signature-refused"),
            Self::SessionEnded => (close_code::POLICY, "session-ended"),
            Self::Malformed => (close_code::INVALID, ""),
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
    /// Closes it, for the reason it sends.
    end: oneshot::Sender<Closing>,
}

/// A connection of `device` while it is held: dropping it lets the connection go.
struct Registration {
    connections: Arc<Connections>,
    device: DeviceId,
    id: u64,
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
        .on_upgrade(move |socket| serve(state, socket, version))
}

/// A connection, once its `auth` proved its device, held as the device's.
struct Proved {
    device: DeviceId,
    account: AccountId,
    /// Lets the connection go when dropped.
    _registration: Registration,
    /// Fires when the session ends or the device is revoked.
    ended: oneshot::Receiver<Closing>,
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
                return close(socket, Closing::ServerError).await;
            }
            let sender = Sender {
                device: proved.device,
                account: proved.account,
                version,
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
/// sends the notices of its account's spaces.
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
            spaces = notices.next() => notify(&mut socket, spaces).await,
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

/// Serves the frame `bytes` from `sender`: answers a `send`, acks a `sync` or an `ack`, and ignores
/// a frame of another type.
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

/// Sends a `space-notice` of each of `spaces` on `socket`.
async fn notify(socket: &mut WebSocket, spaces: Vec<SpaceId>) -> Result<(), Stop> {
    for space in spaces {
        send_frame(socket, &Frame::Push(Push::SpaceNotice { space })).await?;
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
    let session = challenge_session(state, &challenge)
        .await?
        .ok_or(Closing::ChallengeRefused)?;
    // Held before the challenge is used, so that once the use finds the session, its end closes
    // the connection; an end before the use deletes the challenge.
    let held = state.connections.hold(device, session);
    let account = take_challenge(state, &challenge)
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
    let (registration, ended) = held;
    Ok(Some(Proved {
        device,
        account: account.parse().expect("a stored account ID"),
        _registration: registration,
        ended,
    }))
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

/// The SHA-256 of the token of `challenge`'s session, if the server holds the challenge.
async fn challenge_session(state: &AppState, challenge: &[u8]) -> Result<Option<Vec<u8>>, Closing> {
    sqlx::query_as("SELECT session_hash FROM connection_challenges WHERE challenge_hash = $1")
        .bind(Sha256::digest(challenge).to_vec())
        .fetch_optional(&state.db)
        .await
        .map(|found| found.map(|(session,)| session))
        .map_err(|err| failed("read a challenge", err))
}

/// Uses up `challenge`: its session's account, if it is one the server issued, unused, unexpired,
/// to a session that has not ended.
async fn take_challenge(state: &AppState, challenge: &[u8]) -> Result<Option<String>, Closing> {
    let taken: Option<(String, bool)> = sqlx::query_as(
        "DELETE FROM connection_challenges c USING sessions s
         WHERE c.challenge_hash = $1 AND s.token_hash = c.session_hash
         RETURNING s.account_id::text, c.expires_at > now()",
    )
    .bind(Sha256::digest(challenge).to_vec())
    .fetch_optional(&state.db)
    .await
    .map_err(|err| failed("take a challenge", err))?;
    Ok(taken
        .filter(|(_, unexpired)| *unexpired)
        .map(|(account, _)| account))
}

/// The signing key of `device` if it is a `client` device of `account` with a certificate on the
/// server, not revoked.
async fn client_key(
    state: &AppState,
    device: DeviceId,
    account: &str,
) -> Result<Option<[u8; 32]>, Closing> {
    let found: Option<(Vec<u8>,)> = sqlx::query_as(
        "SELECT signing FROM devices
         WHERE id = $1::uuid AND account_id = $2::uuid AND kind = 'client' AND cert IS NOT NULL
           AND NOT revoked",
    )
    .bind(device.to_string())
    .bind(account)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| failed("read a device", err))?;
    Ok(found.and_then(|(signing,)| signing.try_into().ok()))
}

/// The closing when the database fails at `doing`; the failure goes to the server's output.
fn failed(doing: &str, err: sqlx::Error) -> Closing {
    eprintln!("skald-server: cannot {doing}: {err}");
    Closing::ServerError
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
