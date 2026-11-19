//! What the tests share: the bot's tokens and its requests with them, envelopes between alice's
//! laptop and the bot, a channel outside the bot's grant, and how a refused connection or send
//! looks.

use std::time::{Duration, Instant};

use skald_proto::{Cbor, ChannelId, Frame, Header, Push};
use tungstenite::Message;

use crate::api::{Session, assert_refused};
use crate::common::devices::CHALLENGE;
use crate::common::{Response, Server};
use crate::https::{Https, answer_of};
use crate::link::{Link, QUIET, bytes, store_to};
use crate::rosters::channel;
use crate::sse::Sse;
use crate::tokens::{Created, Stage};
use crate::wire::{Answer, Socket, assert_nowhere, envelope, envelopes_of, header};

/// How long a test waits for the server to close a connection or answer a request it ends.
const WAIT: Duration = Duration::from_secs(15);

/// A token of the bot with `permissions`, for `general` until a day from now, from alice's session.
#[track_caller]
pub fn token(stage: &Stage, permissions: &[&str]) -> Created {
    let alice = &stage.f.team.alice;
    stage.create_as(alice, stage.bot.id, &stage.bot_grant(), permissions)
}

/// The fixture's server.
pub fn server(stage: &Stage) -> &Server {
    &stage.f.team.flow.stand.server
}

/// The bot's requests over HTTPS to `server` with `token`.
pub fn https_on(stage: &Stage, server: &Server, token: &str) -> Https {
    Https::on_session(server, &stage.f.team.flow, token, &stage.bot)
}

/// The bot's requests over HTTPS with `token`.
pub fn https(stage: &Stage, token: &str) -> Https {
    https_on(stage, server(stage), token)
}

/// The bot's streams on `server` with `token`.
pub fn sse_on(stage: &Stage, server: &Server, token: &str) -> Sse {
    Sse::on_session(server, &stage.f.team.flow, token, &stage.bot)
}

/// The bot's streams with `token`.
pub fn sse(stage: &Stage, token: &str) -> Sse {
    sse_on(stage, server(stage), token)
}

/// The bot's WebSocket to `server` on a challenge of `token`.
pub fn socket_on(stage: &Stage, server: &Server, token: &str) -> Socket {
    Socket::on_session(server, &stage.f.team.flow, token, &stage.bot)
}

/// The bot's WebSocket on a challenge of `token`.
pub fn socket(stage: &Stage, token: &str) -> Socket {
    socket_on(stage, server(stage), token)
}

/// The bot's WebSocket on a challenge of `token`, as its mailbox's reader sees it.
pub fn link(stage: &Stage, token: &str) -> Link {
    Link::on(socket(stage, token))
}

/// Requests with `token` alone: its bearer token, no proof.
pub fn alone(stage: &Stage, token: &str) -> Session {
    Session::of_token(&stage.f.team.flow, token)
}

/// The response to `POST /session/challenge` with `token`.
pub fn challenge_response(stage: &Stage, token: &str) -> Response {
    alone(stage, token).call("post", CHALLENGE, None)
}

/// A challenge of `token`, which the server must give.
#[track_caller]
pub fn challenge(stage: &Stage, token: &str) -> Vec<u8> {
    https(stage, token).challenge()
}

/// Stores `count` envelopes from alice's laptop to the bot in `general`, without receipts; gives
/// their bytes.
#[track_caller]
pub fn to_bot(stage: &Stage, count: usize) -> Vec<Vec<u8>> {
    let mut alice = Link::of(&stage.f.team.flow, &stage.f.team.alice);
    bytes(&store_to(&mut alice, &stage.f, stage.bot.id, count, false))
}

/// A new envelope from the bot to alice's laptop in `channel` of `acme`: its header and bytes.
pub fn to_alice(stage: &Stage, channel: ChannelId) -> (Header, Vec<u8>) {
    let header = header(stage.space(), channel, stage.f.team.alice.laptop.id);
    let sent = envelope(&header);
    (header, sent)
}

/// Checks that `https` stores a new envelope to alice's laptop in `general`, which her mailbox
/// then holds alone.
#[track_caller]
pub fn assert_sends(stage: &Stage, https: &Https) {
    let (header, sent) = to_alice(stage, stage.f.general);
    https.store(&header, &sent);
    assert_alice_holds(stage, &sent);
}

/// Checks that the mailbox of alice's laptop holds `sent` alone.
#[track_caller]
pub fn assert_alice_holds(stage: &Stage, sent: &[u8]) {
    let alice = stage.f.team.alice.laptop.id;
    assert_eq!(envelopes_of(stage.db(), alice), vec![sent.to_vec()]);
}

/// Stores the next revision of `acme` with a new channel `ops` of alice and bob, which the bot's
/// grants do not list; gives its ID.
pub fn ops(stage: &mut Stage) -> ChannelId {
    let ops = stage.f.team.alice.session.channel(stage.space());
    let members = [&stage.f.team.alice, &stage.f.team.bob];
    let entry = channel(ops, &members);
    stage.f.revise(|body| body.channels.push(entry));
    ops
}

/// Checks that the server refuses the send of `sent`: `response` is `send-refused` or a `4xx` with
/// a problem details object, and the database holds nothing of it; `what` names the case.
#[track_caller]
pub fn assert_send_refused(stage: &Stage, response: &Response, sent: &[u8], what: &str) {
    if response.status == 200 {
        let answer = answer_of(response);
        assert!(
            matches!(answer, Answer::Refused(..)),
            "{what}: refused, not {answer:?}"
        );
    } else {
        assert_refused(response, what);
    }
    assert_nowhere(stage.db(), sent);
}

/// Checks that the server refuses the envelope `sent`, just sent on `socket`: `send-refused`, or it
/// closes the connection; and that the database holds nothing of it. `what` names the case.
#[track_caller]
pub fn assert_socket_send_refused(stage: &Stage, socket: &mut Socket, sent: &[u8], what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(frame))) => match Frame::from_cbor(&frame) {
                Ok(Frame::SendRefused { .. }) => break,
                Ok(Frame::Stored(stored)) => panic!("{what}: stored: {stored:?}"),
                _ => {}
            },
            Ok(Some(Message::Close(_))) | Err(_) => break,
            Ok(Some(_)) => {}
            Ok(None) => panic!("{what}: no answer within {WAIT:?}"),
        }
    }
    assert_nowhere(stage.db(), sent);
}

/// Checks that no `message` or `status` comes on `socket` for `QUIET`; it may close.
#[track_caller]
pub fn assert_no_entry(socket: &mut Socket, what: &str) {
    let deadline = Instant::now() + QUIET;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(frame))) => {
                let pushes = match Frame::from_cbor(&frame) {
                    Ok(Frame::Push(push)) => vec![push],
                    Ok(Frame::Batch(pushes)) => pushes,
                    _ => Vec::new(),
                };
                let entries: Vec<&Push> = pushes
                    .iter()
                    .filter(|push| matches!(push, Push::Message { .. } | Push::Status { .. }))
                    .collect();
                assert!(entries.is_empty(), "{what}: entries came: {entries:?}");
            }
            Ok(Some(Message::Close(_))) | Err(_) | Ok(None) => return,
            Ok(Some(_)) => {}
        }
    }
}

/// Checks that the server does not take `socket`: it closes the connection, with a close frame or
/// without, and sends nothing of a mailbox first.
#[track_caller]
pub fn assert_not_taken(socket: &mut Socket, what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(frame))) => {
                if let Ok(Frame::Batch(_) | Frame::Push(_)) = Frame::from_cbor(&frame) {
                    panic!("{what}: taken, catch-up came");
                }
            }
            Ok(Some(Message::Close(_))) | Err(_) => return,
            Ok(Some(_)) => {}
            Ok(None) => panic!("{what}: still open after {WAIT:?}"),
        }
    }
}
