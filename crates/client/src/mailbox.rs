//! The device's connection to its mailbox (`docs/dev/architecture.md`, "Network";
//! `docs/protocol/delivery.md`, "Frames" and "Mailbox"). It tries a WebSocket, then the mailbox's
//! event stream over SSE, then long-poll, each when the one before does not open or does not end
//! its catch-up within the run's wait for each frame. Over SSE and long-poll the device sends and
//! acks in HTTPS requests, each proving the device. Once a transport works, the run's later
//! connections start with it; a dropped connection reconnects in the same order, from WebSocket.
//!
//! A one-shot run's long-poll asks with a wait of 0 until an answer is empty: the end of its
//! catch-up. `history` frames go over a WebSocket only.
//!
//! A WebSocket or an event stream a run is done with is kept for its next connection as the same
//! device, and the helper's for its next operations ([`crate::helper`]). Catch-up on a WebSocket
//! after its first takes the entries it pushed since, which the client reads as they come; on an
//! event stream it goes over long-poll, which, unlike a later `sync`, ends.
//!
//! A run of `skald recv --wait` watches for what comes between its runs of `skald recv`
//! ([`crate::wait`]): on the connection kept, or, with none, on a long-poll that waits
//! ([`Watched`]).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::time::Duration;

use skald_proto::{Cbor as _, DeviceId, Frame, Push, SpaceId};
use tungstenite::Message;
use tungstenite::stream::MaybeTlsStream;

use crate::connection::{self, Connection, WebSocket};
use crate::device::Keys;
use crate::devices::text;
use crate::events::Events;
use crate::recv::{Got, got, noticed};
use crate::send::{closed, set_read_timeout};
use crate::spaces::Follower;

/// The paths of the WebSocket, the event stream, the long-poll, the send and the ack.
const WEBSOCKET: &str = "/ws";
const EVENTS: &str = "/mailbox/events";
const POLL: &str = "/mailbox/poll";
pub const SEND: &str = "/mailbox/send";
const ACK: &str = "/mailbox/ack";

/// The transports of the device's connection, in the order the client tries them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transport {
    #[default]
    WebSocket,
    Sse,
    LongPoll,
}

/// The device's open connection.
pub struct Mailbox {
    /// Taken only when it closes or is kept.
    link: Option<Link>,
    owner: Owner,
    /// Whether a catch-up on it has ended: the server sends nothing to end another.
    caught_up: bool,
    /// On a WebSocket, the entries it pushed after its catch-up that are not written yet.
    pushed: Vec<Push>,
}

/// How a connection reaches the mailbox.
enum Link {
    WebSocket(WebSocket),
    /// HTTPS requests, with the event stream once open over SSE.
    Https(Option<Events>),
}

/// Whose a connection is: a server, a device, its session and the protocol version spoken.
#[derive(Clone, PartialEq)]
struct Owner {
    server: String,
    device: DeviceId,
    token: String,
    version: std::num::NonZeroU64,
}

impl Owner {
    fn of(follower: &Follower) -> Self {
        Owner {
            server: follower.connection.server().to_owned(),
            device: follower.this,
            token: follower.token.clone(),
            version: follower.connection.version(),
        }
    }
}

/// A connection kept for the run's next one, or the helper's next operation, with the transport it
/// is on.
struct Kept {
    mailbox: Mailbox,
    transport: Transport,
}

thread_local! {
    /// The connection kept.
    static KEPT: RefCell<Option<Kept>> = const { RefCell::new(None) };
    /// Whether a run of this thread read and dropped what came on its connection, which a later
    /// run gets: a run that waits for what comes, and ran before it, missed it.
    static MISSED: Cell<bool> = const { Cell::new(false) };
}

/// Whether a run of this thread dropped what came on its connection since this was last asked.
pub fn missed() -> bool {
    MISSED.with(|missed| missed.replace(false))
}

/// Keeps `push`, which a read of a WebSocket for the answers to what it sent met, in `pushed`, for
/// the next catch-up to write before it acks; a wait looks again, as for what a run dropped.
pub fn keep_push(pushed: &mut Vec<Push>, push: Push) {
    pushed.push(push);
    miss();
}

/// Notes that a run dropped what came on its connection.
fn miss() {
    MISSED.with(|missed| missed.set(true));
}

/// What came on a connection since it was last read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Came {
    #[default]
    Nothing,
    /// Only messages and `space-notice`s, of these spaces.
    In(BTreeSet<SpaceId>),
    /// Anything else too, such as a status, or the connection's end.
    Anything,
}

impl Came {
    /// Adds what `frame`, what a frame from the server gives, brings.
    pub fn add(&mut self, frame: &Got) {
        let spaces = match frame {
            Got::Entries(entries) => (entries.iter())
                .map(|push| match push {
                    Push::Message { envelope, .. } => Some(envelope.envelope().header.space),
                    Push::SpaceNotice { space } => Some(*space),
                    Push::Status { .. } | Push::Unknown(_) => None,
                })
                .collect::<Option<Vec<_>>>(),
            Got::End | Got::Nothing => None,
        };
        *self = match (std::mem::take(self), spaces) {
            (Came::Anything, _) | (_, None) => Came::Anything,
            (Came::Nothing, Some(spaces)) => Came::In(spaces.into_iter().collect()),
            (Came::In(mut all), Some(spaces)) => {
                all.extend(spaces);
                Came::In(all)
            }
        };
    }

    /// Whether it may hold something for a run within the space `space`, or within every space if
    /// `None`.
    pub fn concerns(&self, space: Option<SpaceId>) -> bool {
        match self {
            Came::Nothing => false,
            Came::In(spaces) => space.is_none_or(|space| spaces.contains(&space)),
            Came::Anything => true,
        }
    }
}

/// What came on the kept connection since it was last read; with none kept, or once the
/// connection has ended, which drops it, anything. It reads and drops what came, so the server's
/// pushes, which the next catch-up gets anyway, do not pile up while the helper waits.
pub fn kept_came() -> Came {
    let Some(mut kept) = KEPT.with(|kept| kept.borrow_mut().take()) else {
        return Came::Anything;
    };
    match kept.mailbox.waiting() {
        Some(came) => {
            KEPT.with(|slot| *slot.borrow_mut() = Some(kept));
            came
        }
        None => {
            kept.mailbox.discard();
            Came::Anything
        }
    }
}

/// The transport of the kept connection, if any.
pub fn kept_transport() -> Option<Transport> {
    KEPT.with(|kept| kept.borrow().as_ref().map(|kept| kept.transport))
}

/// Closes the kept connection, if any.
pub fn close_kept() {
    if let Some(kept) = KEPT.with(|kept| kept.borrow_mut().take()) {
        kept.mailbox.discard();
    }
}

/// Gives the kept connection if it is `follower`'s device's and still open, setting the transport
/// it is on.
fn reuse(follower: &mut Follower) -> Option<Mailbox> {
    let mut kept = KEPT.with(|kept| kept.borrow_mut().take())?;
    let waiting = (kept.mailbox.owner == Owner::of(follower))
        .then(|| kept.mailbox.waiting())
        .flatten();
    let Some(came) = waiting else {
        kept.mailbox.discard();
        return None;
    };
    if came != Came::Nothing {
        miss();
    }
    follower.transport = kept.transport;
    Some(kept.mailbox)
}

/// Keeps `link`, a connection a run is done with, for its next one, unless one is kept already or
/// it is not a connection: long-poll holds none, and the next connection tries a WebSocket again.
fn keep(link: Link, owner: Owner, caught_up: bool, pushed: Vec<Push>) {
    let transport = match &link {
        Link::WebSocket(_) => Transport::WebSocket,
        Link::Https(Some(_)) => Transport::Sse,
        Link::Https(None) => return,
    };
    // Once the thread's storage is gone, as when a thread ends, the connection closes.
    let _ = KEPT.try_with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.is_none() {
            *kept = Some(Kept {
                mailbox: Mailbox {
                    link: Some(link),
                    owner,
                    caught_up,
                    pushed,
                },
                transport,
            });
        }
    });
}

/// Keeps the connection a run drops ([`keep`]).
impl Drop for Mailbox {
    fn drop(&mut self) {
        if let Some(link) = self.link.take() {
            let pushed = std::mem::take(&mut self.pushed);
            keep(link, self.owner.clone(), self.caught_up, pushed);
        }
    }
}

/// What writes the entries of a frame after the cursor it is given, with the new cursor, and gives
/// that cursor.
type Write<'w> = &'w mut dyn FnMut(&mut Follower, Vec<Push>, u64) -> Result<u64, String>;

/// Why a catch-up stopped before its end.
enum Stop {
    /// The transport did not open, or did not end its catch-up in time: the next one may.
    Next,
    /// It failed for a reason another transport does not change.
    Failed(String),
}

/// Whether `err`, of opening a transport, is one a proxy that blocks it gives: an answer that is
/// not the server's, none in time, or a connection that breaks.
fn blocked(err: &connection::Error) -> bool {
    matches!(
        err,
        connection::Error::Unexpected(_)
            | connection::Error::Timeout(_)
            | connection::Error::Unreachable(_)
    )
}

impl Mailbox {
    /// Opens the device's connection with its `keys`: the one kept, if any ([`keep`]); otherwise a
    /// WebSocket, unless one did not work earlier in the run; otherwise HTTPS, the event stream
    /// opening at catch-up.
    pub fn open(follower: &mut Follower, keys: &Keys) -> Result<Self, String> {
        if let Some(kept) = reuse(follower) {
            return Ok(kept);
        }
        let owner = Owner::of(follower);
        let link = |link| Mailbox {
            link: Some(link),
            owner,
            caught_up: false,
            pushed: Vec::new(),
        };
        if follower.transport == Transport::WebSocket {
            let (token, connection) = (&follower.token, &mut follower.connection);
            match connection.open_websocket_as(WEBSOCKET, token, keys.id(), keys.signing()) {
                Ok(socket) => return Ok(link(Link::WebSocket(socket))),
                Err(err) if blocked(&err) => follower.transport = Transport::Sse,
                Err(err) => return Err(err.to_string()),
            }
        }
        Ok(link(Link::Https(None)))
    }

    /// The connection's WebSocket, if it is one, and where a read of it keeps each push it meets
    /// ([`keep_push`]).
    pub fn websocket(&mut self) -> Option<(&mut WebSocket, &mut Vec<Push>)> {
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => Some((socket, &mut self.pushed)),
            _ => None,
        }
    }

    /// Closes the connection rather than keep it, as after a failure: a WebSocket with a close
    /// frame.
    pub fn discard(mut self) {
        if let Some(Link::WebSocket(mut socket)) = self.link.take() {
            let _ = socket.close(None);
            let _ = socket.flush();
        }
    }

    /// Reads whatever came on the connection, keeping what a WebSocket pushed for its next
    /// catch-up and dropping the rest: what came, `None` once the connection has closed or broken.
    fn waiting(&mut self) -> Option<Came> {
        let mut came = Came::Nothing;
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => {
                let (messages, open) = read_now(socket);
                for message in messages {
                    if let Message::Binary(bytes) = message {
                        let entries = got(Frame::from_cbor(&bytes).ok());
                        came.add(&entries);
                        if let Got::Entries(entries) = entries {
                            self.pushed.extend(entries);
                        }
                    }
                }
                open.then_some(came)
            }
            Some(Link::Https(Some(events))) => loop {
                match events.next(Duration::ZERO) {
                    Ok(Some(frame)) => came.add(&got(Frame::from_cbor(&frame).ok())),
                    Ok(None) => return Some(came),
                    Err(_) => return None,
                }
            },
            Some(Link::Https(None)) => Some(Came::Nothing),
            None => None,
        }
    }

    /// Gets the mailbox after the device's cursor until catch-up ends, waiting for each frame for
    /// `timeout`; `write` writes the entries of each frame after the cursor it is given, with the
    /// new cursor, and gives it. Falls back to the next transport as the module says; gives the
    /// connection it ended on. On a WebSocket whose catch-up has ended, it takes what it pushed
    /// since; on another connection whose catch-up has ended, it goes over long-poll.
    pub fn catch_up(
        mut self,
        follower: &mut Follower,
        keys: &Keys,
        timeout: Duration,
        mut write: impl FnMut(&mut Follower, Vec<Push>, u64) -> Result<u64, String>,
    ) -> Result<Self, String> {
        loop {
            let cursor = follower.state.cursor().map_err(text)?;
            let caught_up = self.caught_up;
            let pushed = std::mem::take(&mut self.pushed);
            let stopped = match self.link.as_mut() {
                Some(Link::WebSocket(socket)) if caught_up => {
                    from_pushed(socket, follower, (cursor, pushed), &mut write)
                }
                _ if caught_up => over_long_poll(follower, keys, cursor, &mut write),
                Some(Link::WebSocket(socket)) => {
                    over_websocket(socket, follower, (cursor, timeout), &mut write)
                }
                Some(Link::Https(events)) if follower.transport != Transport::LongPoll => {
                    over_sse(events, follower, keys, (cursor, timeout), &mut write)
                }
                _ => over_long_poll(follower, keys, cursor, &mut write),
            };
            match stopped {
                Ok(()) => {
                    self.caught_up = true;
                    return Ok(self);
                }
                Err(Stop::Failed(err)) => return Err(err),
                Err(Stop::Next) => {
                    follower.transport = match follower.transport {
                        Transport::WebSocket => Transport::Sse,
                        Transport::Sse | Transport::LongPoll => Transport::LongPoll,
                    };
                    // A connection that failed takes no close frame.
                    self.link = Some(Link::Https(None));
                    self.caught_up = false;
                }
            }
        }
    }

    /// The spaces of the `space-notice` frames that came on the connection after catch-up and wait
    /// to be read, or, over long-poll, that a long-poll with a wait of 0 gives; the next run gets
    /// the other entries, which a WebSocket keeps.
    pub fn waiting_notices(&mut self, follower: &mut Follower, keys: &Keys) -> Vec<SpaceId> {
        let mut frames = Vec::new();
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => {
                for message in read_now(socket).0 {
                    if let Message::Binary(bytes) = message {
                        frames.push(bytes.to_vec());
                    }
                }
            }
            Some(Link::Https(Some(events))) => {
                while let Ok(Some(frame)) = events.next(Duration::ZERO) {
                    frames.push(frame);
                }
            }
            Some(Link::Https(None)) if follower.transport == Transport::LongPoll => {
                if let Ok(cursor) = follower.state.cursor()
                    && let Ok(frame) = poll(follower, keys, cursor)
                {
                    frames.push(frame);
                }
            }
            Some(Link::Https(None)) | None => {}
        }
        let websocket = matches!(self.link, Some(Link::WebSocket(_)));
        let mut spaces = Vec::new();
        for frame in frames {
            if let Got::Entries(entries) = got(Frame::from_cbor(&frame).ok()) {
                spaces.extend(noticed(&entries));
                if entries
                    .iter()
                    .any(|push| !matches!(push, Push::SpaceNotice { .. }))
                {
                    miss();
                }
                if websocket {
                    self.pushed.extend(entries);
                }
            }
        }
        spaces
    }
}

/// The messages that came on `socket` and wait to be read, read without waiting, and whether it
/// is still open: neither closed nor broken.
fn read_now(socket: &mut WebSocket) -> (Vec<Message>, bool) {
    let mut messages = Vec::new();
    if set_nonblocking(socket, true).is_err() {
        return (messages, false);
    }
    let open = loop {
        match socket.read() {
            Ok(Message::Close(_)) => break false,
            Ok(message) => messages.push(message),
            Err(err) => break crate::send::timed_out(&err),
        }
    };
    (messages, set_nonblocking(socket, false).is_ok() && open)
}

/// Sets whether a read or write of `socket` returns at once rather than wait.
fn set_nonblocking(socket: &mut WebSocket, nonblocking: bool) -> std::io::Result<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_nonblocking(nonblocking),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_nonblocking(nonblocking),
        _ => Ok(()),
    }
}

/// Why a catch-up's HTTPS request failed, by `err`: if it got no answer within the run's wait,
/// that the next run gets the rest.
fn failed(err: &connection::Error) -> String {
    match err {
        connection::Error::Timeout(timeout) => format!(
            "the server sent nothing within {} s before the end of the mailbox; the next skald \
             recv gets the rest",
            timeout.as_secs()
        ),
        err => err.to_string(),
    }
}

/// Sends `frame` to `path` in an HTTPS request as the device of `keys`, its answer awaited for
/// `wait` more than the request timeout; gives the answer's body.
pub fn request(
    follower: &mut Follower,
    keys: &Keys,
    path: &str,
    frame: &[u8],
    wait: Duration,
) -> Result<Vec<u8>, connection::Error> {
    request_on(
        &mut follower.connection,
        (&follower.token, keys),
        path,
        frame,
        wait,
    )
}

/// [`request`] on `connection`, on the device session of `token`.
fn request_on(
    connection: &mut Connection,
    (token, keys): (&str, &Keys),
    path: &str,
    frame: &[u8],
    wait: Duration,
) -> Result<Vec<u8>, connection::Error> {
    let proof = connection.request_proof(token, keys.id(), keys.signing())?;
    connection.post_frame_as(path, (token, &proof), frame, wait)
}

/// A long-poll after `cursor`, acking it, that waits up to `wait` for what comes: the frame it
/// answers.
fn poll_on(
    connection: &mut Connection,
    session: (&str, &Keys),
    cursor: u64,
    wait: Duration,
) -> Result<Vec<u8>, connection::Error> {
    let frame = Frame::Sync { cursor }.to_cbor();
    let path = format!("{POLL}?wait={}", wait.as_secs());
    request_on(connection, session, &path, &frame, wait)
}

/// A long-poll with a wait of 0 after `cursor`, acking it: the frame it answers.
fn poll(follower: &mut Follower, keys: &Keys, cursor: u64) -> Result<Vec<u8>, connection::Error> {
    let session = (follower.token.as_str(), keys);
    poll_on(&mut follower.connection, session, cursor, Duration::ZERO)
}

/// What a run of `skald recv --wait` that keeps no connection watches with: a connection of its
/// own to the device's server, its session and keys, and the cursor its run ended with.
pub struct Watched {
    connection: Connection,
    token: String,
    keys: Keys,
    cursor: u64,
}

impl Watched {
    /// What `follower`, at the end of a run of `skald recv --wait`, watches with; `None` if its
    /// keys, cursor or server do not read.
    pub fn of(follower: &Follower) -> Option<Self> {
        Some(Watched {
            connection: follower.connection.again().ok()?,
            token: follower.token.clone(),
            keys: follower.keys().ok()?,
            cursor: follower.state.cursor().ok()?,
        })
    }

    /// A long-poll after the cursor that waits up to `wait`: what it gave, [`Got::End`] if
    /// nothing came.
    pub fn poll(&mut self, wait: Duration) -> Result<Got, connection::Error> {
        let session = (self.token.as_str(), &self.keys);
        let answer = poll_on(&mut self.connection, session, self.cursor, wait)?;
        Ok(got(Frame::from_cbor(&answer).ok()))
    }
}

/// Catch-up over `socket`: states `cursor` in `sync`, then writes each frame's entries with
/// `write` and acks them, waiting for each frame for `timeout`. A connection that breaks, or
/// sends nothing in time, before the end gives way to the next transport; one the server closes
/// fails.
fn over_websocket(
    socket: &mut WebSocket,
    follower: &mut Follower,
    (mut cursor, timeout): (u64, Duration),
    write: Write<'_>,
) -> Result<(), Stop> {
    let send = |socket: &mut WebSocket, frame: &Frame| {
        (socket.send(Message::Binary(frame.to_cbor().into()))).map_err(|_| Stop::Next)
    };
    send(socket, &Frame::Sync { cursor })?;
    loop {
        set_read_timeout(socket, timeout).map_err(|_| Stop::Next)?;
        let entries = match socket.read() {
            Ok(Message::Binary(bytes)) => match got(Frame::from_cbor(&bytes).ok()) {
                Got::Entries(entries) => entries,
                Got::End => return Ok(()),
                Got::Nothing => continue,
            },
            Ok(Message::Close(frame)) => return Err(Stop::Failed(closed(frame.as_ref()))),
            Ok(_) => continue,
            // Nothing in time, or a broken connection.
            Err(_) => return Err(Stop::Next),
        };
        let written = write(follower, entries, cursor).map_err(Stop::Failed)?;
        if let Some(seq) = std::num::NonZeroU64::new(written).filter(|_| written > cursor) {
            send(socket, &Frame::Ack { seq })?;
        }
        cursor = written;
    }
}

/// Catch-up over the event stream `events`, opening it after `cursor` if it is not open: writes
/// each event's entries with `write`, waiting for each frame for `timeout`, and acks them in one
/// HTTPS request at the end. A stream that does not open, breaks or sends nothing in time before
/// the end gives way to long-poll.
fn over_sse(
    events: &mut Option<Events>,
    follower: &mut Follower,
    keys: &Keys,
    (mut cursor, timeout): (u64, Duration),
    write: Write<'_>,
) -> Result<(), Stop> {
    if events.is_none() {
        let connection = &mut follower.connection;
        let proof = (connection.request_proof(&follower.token, keys.id(), keys.signing()))
            .map_err(|err| Stop::Failed(err.to_string()))?;
        match connection.open_events_as(EVENTS, (&follower.token, &proof), cursor) {
            Ok(stream) => *events = Some(stream),
            Err(err) if blocked(&err) => return Err(Stop::Next),
            Err(err) => return Err(Stop::Failed(err.to_string())),
        }
    }
    let stream = events.as_mut().expect("the stream is open");
    let opened = cursor;
    loop {
        let frame = match stream.next(timeout) {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(_) => return Err(Stop::Next),
        };
        match got(Frame::from_cbor(&frame).ok()) {
            Got::Entries(entries) => {
                cursor = write(follower, entries, cursor).map_err(Stop::Failed)?
            }
            Got::End => break,
            Got::Nothing => {}
        }
    }
    let Some(seq) = std::num::NonZeroU64::new(cursor).filter(|_| cursor > opened) else {
        return Ok(());
    };
    let ack = Frame::Ack { seq }.to_cbor();
    request(follower, keys, ACK, &ack, Duration::ZERO)
        .map(drop)
        .map_err(|err| Stop::Failed(failed(&err)))
}

/// Catch-up on `socket` after its first: writes with `write` the entries after `cursor` of
/// `pushed`, those it pushed since, and acks up to the last written, which the server takes again
/// harmlessly if none was new. A WebSocket pushes each entry stored after its catch-up, in `seq`
/// order, so nothing else waits. An ack that does not go out is left to the next connection's
/// cursor.
fn from_pushed(
    socket: &mut WebSocket,
    follower: &mut Follower,
    (cursor, pushed): (u64, Vec<Push>),
    write: Write<'_>,
) -> Result<(), Stop> {
    let written = write(follower, pushed, cursor).map_err(Stop::Failed)?;
    if let Some(seq) = std::num::NonZeroU64::new(written) {
        let _ = socket.send(Message::Binary(Frame::Ack { seq }.to_cbor().into()));
    }
    Ok(())
}

/// Catch-up over long-poll: asks with a wait of 0 after the cursor, which acks what it covers, and
/// writes each answer's entries with `write`, until an answer is empty.
fn over_long_poll(
    follower: &mut Follower,
    keys: &Keys,
    mut cursor: u64,
    write: Write<'_>,
) -> Result<(), Stop> {
    loop {
        let answer = poll(follower, keys, cursor).map_err(|err| Stop::Failed(failed(&err)))?;
        match Frame::from_cbor(&answer) {
            Ok(Frame::Batch(entries)) if entries.is_empty() => return Ok(()),
            Ok(Frame::Batch(entries)) => {
                cursor = write(follower, entries, cursor).map_err(Stop::Failed)?;
            }
            _ => {
                return Err(Stop::Failed(
                    "unexpected answer from the server: a long-poll answered without a batch"
                        .to_owned(),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request that got no answer in time says the server sent nothing in time; any other
    /// failure says what it was.
    #[test]
    fn failed_request_says_whether_it_timed_out() {
        let silent = "the server sent nothing within 4 s before the end of the mailbox; the next \
                      skald recv gets the rest";

        let timeout = connection::Error::Timeout(Duration::from_secs(4));
        assert_eq!(failed(&timeout), silent);
        let reset = connection::Error::Unreachable("connection reset".to_owned());
        assert_eq!(failed(&reset), reset.to_string());
    }

    /// Only what a proxy that blocks a transport gives lets the next one try: no answer of the
    /// server's, none in time, or a broken connection; never an answer of the server's, a version
    /// it refuses or a proxy that gives no tunnel at all.
    #[test]
    fn only_a_blocking_proxys_failure_tries_the_next_transport() {
        let blocking = [
            connection::Error::Unexpected("HTTP 403".to_owned()),
            connection::Error::Timeout(Duration::from_secs(4)),
            connection::Error::Unreachable("connection reset".to_owned()),
        ];
        assert!(blocking.iter().all(blocked));
        let other = [
            connection::Error::Refused {
                status: 503,
                kind: "https://skald.so/problems/server-error".to_owned(),
                detail: "down".to_owned(),
            },
            connection::Error::TooOld {
                supported: vec![9],
                speaks: skald_proto::VersionRange::default(),
            },
            connection::Error::Proxy {
                proxy: "proxy.test:3128".to_owned(),
                problem: "it refused the tunnel with 403 Forbidden".to_owned(),
            },
        ];
        assert!(!other.iter().any(blocked));
    }

    /// What came is the spaces of the messages and notices that came, or anything once a status, a
    /// frame that does not decode or a frame of another kind came.
    #[test]
    fn what_came_is_its_spaces_or_anything() {
        use skald_proto::{
            ChannelId, DeliveryStatus, Envelope, Header, MessageId, RawEnvelope, Scheme,
            ServerStatus, Unknown,
        };
        let seq = std::num::NonZeroU64::MIN;
        let message = |space| {
            let header = Header {
                v: std::num::NonZeroU64::MIN,
                space,
                channel: ChannelId::generate(),
                msg_id: MessageId::generate(),
                scheme: Scheme::Mls,
                recipients: None,
                ttl: None,
                unknown: Unknown::default(),
            };
            let envelope = Envelope {
                header,
                payload: vec![1],
            };
            let envelope = RawEnvelope::from_bytes(envelope.to_cbor()).unwrap();
            Frame::Push(Push::Message { seq, envelope }).to_cbor()
        };
        let (first, second, other) = (
            SpaceId::generate(),
            SpaceId::generate(),
            SpaceId::generate(),
        );
        let notice = Frame::Push(Push::SpaceNotice { space: second }).to_cbor();

        let add = |came: &mut Came, frame: &[u8]| came.add(&got(Frame::from_cbor(frame).ok()));
        let mut came = Came::default();
        assert!(!came.concerns(None));
        add(&mut came, &message(first));
        add(
            &mut came,
            &Frame::Batch(vec![Push::SpaceNotice { space: second }]).to_cbor(),
        );
        add(&mut came, &notice);
        assert_eq!(came, Came::In([first, second].into()));
        assert!(came.concerns(Some(first)) && came.concerns(Some(second)) && came.concerns(None));
        assert!(!came.concerns(Some(other)));

        let status = DeliveryStatus {
            msg_id: MessageId::generate(),
            device: DeviceId::from_signing_key(&[2; 32]),
            status: ServerStatus::Delivered,
            timestamp: 1_800_000_000,
            unknown: Unknown::default(),
        };
        for frame in [
            Frame::Push(Push::Status { seq, status }).to_cbor(),
            vec![0xff],
            Frame::Batch(Vec::new()).to_cbor(),
        ] {
            let mut came = Came::In([first].into());
            add(&mut came, &frame);
            assert_eq!(came, Came::Anything);
            add(&mut came, &message(first));
            assert!(came.concerns(Some(other)));
        }
    }

    /// Closing a WebSocket rather than keeping it sends its close frame.
    #[test]
    fn closing_a_websocket_sends_its_close_frame() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let address = listener.local_addr().unwrap();
        let served = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the WebSocket");
            let mut socket = tungstenite::accept(stream).expect("a WebSocket");
            socket.read().expect("a message")
        });
        let stream = std::net::TcpStream::connect(address).expect("connect");
        let url = format!("ws://{address}/ws");
        let plain = tungstenite::stream::MaybeTlsStream::Plain(stream);
        let (socket, _) = tungstenite::client(url.as_str(), plain).expect("a WebSocket");

        let mailbox = Mailbox {
            link: Some(Link::WebSocket(socket)),
            owner: Owner {
                server: url,
                device: DeviceId::from_signing_key(&[1; 32]),
                token: String::new(),
                version: std::num::NonZeroU64::MIN,
            },
            caught_up: false,
            pushed: Vec::new(),
        };
        mailbox.discard();

        assert!(matches!(served.join().unwrap(), Message::Close(_)));
    }
}
