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
//! device, and the helper's for its next operations ([`crate::helper`]); catch-up on one after its
//! first goes over long-poll, which, unlike a later `sync`, ends.

use std::cell::RefCell;
use std::time::Duration;

use skald_proto::{Cbor as _, DeviceId, Frame, Push, SpaceId};
use tungstenite::Message;

use crate::connection::{self, WebSocket};
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
}

/// Reads what came on the kept connection, if any, so the server's pushes, which the next catch-up
/// gets anyway, do not pile up while the helper waits; a connection that broke or closed is
/// dropped.
pub fn drain_kept() {
    let kept = KEPT.with(|kept| kept.borrow_mut().take());
    if let Some(mut kept) = kept {
        if kept.mailbox.drained() {
            KEPT.with(|slot| *slot.borrow_mut() = Some(kept));
        } else {
            kept.mailbox.discard();
        }
    }
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
    if kept.mailbox.owner != Owner::of(follower) || !kept.mailbox.drained() {
        kept.mailbox.discard();
        return None;
    }
    follower.transport = kept.transport;
    Some(kept.mailbox)
}

/// Keeps `link`, a connection a run is done with, for its next one, unless one is kept already or
/// it is not a connection: long-poll holds none, and the next connection tries a WebSocket again.
fn keep(link: Link, owner: Owner, caught_up: bool) {
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
            keep(link, self.owner.clone(), self.caught_up);
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

    /// The connection's WebSocket, if it is one.
    pub fn websocket(&mut self) -> Option<&mut WebSocket> {
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => Some(socket),
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

    /// Reads and drops whatever came on the connection; whether it is still open.
    fn drained(&mut self) -> bool {
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => {
                if set_read_timeout(socket, Duration::from_millis(1)).is_err() {
                    return false;
                }
                loop {
                    match socket.read() {
                        Ok(Message::Close(_)) => return false,
                        Ok(_) => {}
                        Err(err) => return crate::send::timed_out(&err),
                    }
                }
            }
            Some(Link::Https(Some(events))) => loop {
                match events.next(Duration::ZERO) {
                    Ok(Some(_)) => {}
                    Ok(None) => return true,
                    Err(_) => return false,
                }
            },
            Some(Link::Https(None)) => true,
            None => false,
        }
    }

    /// Gets the mailbox after the device's cursor until catch-up ends, waiting for each frame for
    /// `timeout`; `write` writes the entries of each frame after the cursor it is given, with the
    /// new cursor, and gives it. Falls back to the next transport as the module says; gives the
    /// connection it ended on. On a connection whose catch-up has ended, it goes over long-poll.
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
            let stopped = match self.link.as_mut() {
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
    /// to be read, or, over long-poll, that a long-poll with a wait of 0 gives; the other frames are
    /// dropped, and the next run gets their entries.
    pub fn waiting_notices(&mut self, follower: &mut Follower, keys: &Keys) -> Vec<SpaceId> {
        let mut frames = Vec::new();
        match self.link.as_mut() {
            Some(Link::WebSocket(socket)) => {
                if set_read_timeout(socket, Duration::from_millis(1)).is_ok() {
                    while let Ok(message) = socket.read() {
                        if let Message::Binary(bytes) = message {
                            frames.push(bytes.to_vec());
                        }
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
        let mut spaces = Vec::new();
        for frame in frames {
            if let Got::Entries(entries) = got(Frame::from_cbor(&frame).ok()) {
                spaces.extend(noticed(&entries));
            }
        }
        spaces
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
    let connection = &mut follower.connection;
    let proof = connection.request_proof(&follower.token, keys.id(), keys.signing())?;
    connection.post_frame_as(path, (&follower.token, &proof), frame, wait)
}

/// A long-poll with a wait of 0 after `cursor`, acking it: the frame it answers.
fn poll(follower: &mut Follower, keys: &Keys, cursor: u64) -> Result<Vec<u8>, connection::Error> {
    let frame = Frame::Sync { cursor }.to_cbor();
    request(
        follower,
        keys,
        &format!("{POLL}?wait=0"),
        &frame,
        Duration::ZERO,
    )
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
}
