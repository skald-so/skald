//! A token's connections and waiting requests end when it ends, on every instance, as at a
//! session's end: its WebSocket closes, a waiting long-poll answers with a refusal, and its stream
//! closes.

use std::thread;
use std::time::{Duration, Instant};

use crate::bot::{https_on, socket_on, sse_on, token};
use crate::common::Server;
use crate::common::device_flow::text;
use crate::https::{LONG, Waiting, assert_problem, batch_of};
use crate::kit::{CLOSE, assert_closes_within, assert_open};
use crate::sse::{Event, Stream};
use crate::tokens::{RECEIVE, SEND, Stage, grant};
use crate::wire::{Socket, now};

/// How soon a token's connections end once a revision drops its entry or it expires.
const END: Duration = Duration::from_secs(3);

/// The bot's open WebSocket, waiting long-poll and stream on `server` with `token`.
fn connected(stage: &Stage, server: &Server, token: &str) -> (Socket, Waiting, Stream) {
    let mut socket = socket_on(stage, server, token);
    assert_open(&mut socket);
    let https = https_on(stage, server, token);
    // The first long-poll answers at once with the notices of the spaces stored before it.
    assert_eq!(https.poll_now(0).entries, Vec::new(), "the mailbox");
    let waiting = https.start_poll(0, LONG);
    let sse = sse_on(stage, server, token);
    let mut stream = sse.open(Some(0));
    assert_eq!(stream.catch_up(), Vec::new(), "the stream's catch-up");
    (socket, waiting, stream)
}

/// Checks that the WebSocket closes, the long-poll is answered `401` and the stream closes, each
/// within `within` of `since`; `what` names the case. With `notices`, the end comes with a new
/// revision, so the long-poll may answer its `space-notice` instead, with no entry, and the stream
/// may send it before it closes.
#[track_caller]
fn assert_ended(
    (mut socket, waiting, mut stream): (Socket, Waiting, Stream),
    since: Instant,
    within: Duration,
    notices: bool,
    what: &str,
) {
    let deadline = since + within;
    assert_closes_within(
        &mut socket,
        since,
        within,
        &format!("{what}: the WebSocket"),
    );
    let (response, at) = waiting.answer();
    if notices && response.status == 200 {
        let batch = batch_of(&response);
        assert_eq!(batch.entries, Vec::new(), "{what}: the long-poll's entries");
    } else {
        assert_problem(&response, 401, None);
    }
    let took = at.saturating_duration_since(since);
    assert!(
        took <= within,
        "{what}: the long-poll answered {took:?} after, not within {within:?}"
    );
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match stream.next(left.max(Duration::from_millis(100))) {
            Some((Event::Closed, _)) => return,
            Some((Event::KeepAlive, _)) => {}
            Some((Event::Notice(_), _)) if notices => {}
            Some((event, _)) => panic!("{what}: {event:?} on the stream before it closed"),
            None => panic!("{what}: the stream still open {within:?} after"),
        }
    }
}

/// The bot's WebSocket, waiting long-poll and stream on another instance end within a second of
/// alice's session revoking the token on the first one; another token of the bot connects there
/// still.
#[test]
fn a_tokens_connections_to_another_instance_end_within_a_second_of_its_revocation() {
    let stage = Stage::new(3_545_600_001);
    let revoked = token(&stage, &[SEND, RECEIVE]);
    let kept = token(&stage, &[SEND, RECEIVE]);
    let other = stage.f.team.flow.stand.instance(&[], &[]);
    let connections = connected(&stage, &other, &revoked.token);

    let response = stage.revoke_as(&stage.f.team.alice, &revoked.id);
    let since = Instant::now();
    assert_eq!(response.status, 204, "revoked: {}", text(&response));
    assert_ended(connections, since, CLOSE, false, "revoked");
    assert_open(&mut socket_on(&stage, &other, &kept.token));
}

/// The bot's connections end once a revision drops its service agent entry.
#[test]
fn a_tokens_connections_end_once_a_revision_drops_its_entry() {
    let mut stage = Stage::new(3_545_600_011);
    let created = token(&stage, &[SEND, RECEIVE]);
    let connections = connected(&stage, &stage.f.team.flow.stand.server, &created.token);

    let bot = stage.bot.id;
    stage
        .f
        .revise(|body| body.service_agents.retain(|agent| agent.device != bot));
    let since = Instant::now();
    assert_ended(connections, since, END, true, "its entry dropped");
}

/// The bot's connections on a token whose grant expires 8 seconds after it is signed end at its
/// expiry.
#[test]
fn a_tokens_connections_end_at_its_expiry() {
    let stage = Stage::new(3_545_600_021);
    let alice = &stage.f.team.alice;
    let expires = now() + 8;
    let soon = grant(stage.bot.id, &[stage.f.general], &alice.laptop, expires);
    let created = stage.create_as(alice, stage.bot.id, &soon, &[SEND, RECEIVE]);
    let connections = connected(&stage, &stage.f.team.flow.stand.server, &created.token);

    while now() <= expires {
        thread::sleep(Duration::from_millis(100));
    }
    let since = Instant::now();
    assert_ended(connections, since, END, false, "expired");
}
