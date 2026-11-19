//! What a token's connections and reads reach besides its permissions: only the notices of its
//! space and no `link-notice`, history uploads in its channels only, its channel's history as its
//! device's account reads it, and, at its expiry, the end of its own connections alone.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{Cbor as _, ChannelId, ChannelMember, Frame, Push};
use tungstenite::Message;

use crate::bot::{alone, https, link, ops, socket, to_bot, token};
use crate::common::Response;
use crate::common::device_flow::text;
use crate::common::devices::assert_taken;
use crate::group::formed;
use crate::history::{Answer, ENTRIES, WRAPS, attempt, entry, given, stores, wrap};
use crate::https::Https;
use crate::kit::{assert_closes_within, assert_open};
use crate::link::{QUIET, WAIT, messages};
use crate::link_kit::{fresh_session, hello};
use crate::rosters::first;
use crate::tokens::{Created, RECEIVE, SEND, Stage, grant};
use crate::wire::{Socket, assert_nowhere, now};

/// How soon a token's connections end at its expiry.
const END: Duration = Duration::from_secs(3);

/// A token of the bot with the send and receive permissions, from alice's session, on a grant for
/// `channels` until `expires`.
fn token_for(stage: &Stage, channels: &[ChannelId], expires: u64) -> Created {
    let alice = &stage.f.team.alice;
    let signed = grant(stage.bot.id, channels, &alice.laptop, expires);
    stage.create_as(alice, stage.bot.id, &signed, &[SEND, RECEIVE])
}

/// The bot's long-poll with its token gets the `space-notice` of a new revision of `acme`, and none
/// of a space of alice's account, the bot's, that the token is not of.
#[test]
fn a_tokens_long_poll_gets_the_notices_of_its_space_only() {
    let mut stage = Stage::new(3_545_700_001);
    let created = token(&stage, &[RECEIVE]);
    let https = https(&stage, &created.token);
    // The first long-poll answers the notices of the spaces stored before it.
    https.poll_now(0);

    let alice = &stage.f.team.alice;
    let initech = alice.create_space("initech");
    assert_taken(
        &alice
            .session
            .store(initech, &alice.sign(&first(initech, "initech", alice))),
    );
    assert_eq!(https.poll_now(0).notices, Vec::new(), "another space's");

    stage.f.revise(|_| {});
    assert_eq!(
        https.poll_now(0).notices,
        vec![stage.space()],
        "a revision of acme"
    );
}

/// The pushes of `frame`'s bytes: a push alone, or a `batch`'s; `None` for an empty `batch`, the
/// end of catch-up.
fn pushes_of(bytes: &[u8]) -> Option<Vec<Push>> {
    match Frame::from_cbor(bytes) {
        Ok(Frame::Push(push)) => Some(vec![push]),
        Ok(Frame::Batch(pushes)) if pushes.is_empty() => None,
        Ok(Frame::Batch(pushes)) => Some(pushes),
        _ => Some(Vec::new()),
    }
}

/// What `socket` gets by `deadline`: whether a `link-notice` comes, then `true`, or the end of
/// catch-up, then `false`; `false` if neither comes. Fails if the connection closes.
#[track_caller]
fn link_notice_by(socket: &mut Socket, deadline: Instant) -> bool {
    loop {
        match socket.next(deadline) {
            Ok(Some(Message::Binary(bytes))) => match pushes_of(&bytes) {
                Some(pushes) if pushes.contains(&Push::LinkNotice) => return true,
                Some(_) => {}
                None => return false,
            },
            Ok(Some(Message::Close(frame))) => panic!("the server closed it: {frame:?}"),
            Ok(Some(_)) => {}
            Ok(None) => return false,
            Err(err) => panic!("the connection failed: {err}"),
        }
    }
}

/// Whether the long-poll answer `response`, a `batch`, holds a `link-notice`.
#[track_caller]
fn polled_link_notice(https: &Https) -> bool {
    let response: Response = https.poll_request(0, 0, Some(https.proof())).call();
    assert_eq!(response.status, 200, "{}", text(&response));
    pushes_of(&response.body).is_some_and(|pushes| pushes.contains(&Push::LinkNotice))
}

/// The bot holds a WebSocket and long-polls with a token of the receive permission, as does
/// alice's laptop with her session. A run of alice's uncertified device opens: alice's laptop gets
/// a `link-notice` on both, and the bot's token on neither.
#[test]
fn a_tokens_connections_get_no_link_notice() {
    let stage = Stage::new(3_545_700_011);
    let created = token(&stage, &[RECEIVE]);
    let alice = &stage.f.team.alice;
    let (bot_https, alice_https) = (
        https(&stage, &created.token),
        Https::of(&stage.f.team.flow, alice),
    );
    let mut bot_socket = socket(&stage, &created.token);
    let mut alice_socket = Socket::of(&stage.f.team.flow, alice);
    for socket in [&mut bot_socket, &mut alice_socket] {
        socket.sync(0);
        assert!(
            !link_notice_by(socket, Instant::now() + WAIT),
            "before the hello"
        );
    }
    for https in [&bot_https, &alice_https] {
        assert!(!polled_link_notice(https), "a long-poll before the hello");
    }

    let opened = alice
        .session
        .send_hello(alice.uncertified.id, &hello(&fresh_session()));
    assert_taken(&opened);

    assert!(
        link_notice_by(&mut alice_socket, Instant::now() + WAIT),
        "alice's WebSocket"
    );
    assert!(polled_link_notice(&alice_https), "alice's long-poll");
    assert!(
        !link_notice_by(&mut bot_socket, Instant::now() + QUIET),
        "the token's WebSocket"
    );
    assert!(!polled_link_notice(&bot_https), "the token's long-poll");
}

/// The bot, a member of `general`'s group, uploads an entry of `general` on a WebSocket of a token
/// for `general`; the same entry on a WebSocket of a token for `ops` alone is refused with
/// `not-a-member` and stored nowhere.
#[test]
fn a_tokens_device_uploads_history_only_in_the_tokens_channels() {
    let (mut stage, _laptop, _bot) = formed(3_545_700_011);
    let ops = ops(&mut stage);
    let for_ops = token_for(&stage, &[ops], now() + 86_400);
    let for_general = token(&stage, &[SEND, RECEIVE]);
    let (space, general) = (stage.space(), stage.f.general);
    let item = entry(general, 1);

    let answer = attempt(&mut socket(&stage, &for_ops.token), space, general, &item);
    assert_eq!(
        answer,
        Some(Answer::Refused("not-a-member".to_owned())),
        "on a token for ops alone"
    );
    assert_nowhere(stage.db(), item.bytes());

    let answer = attempt(
        &mut socket(&stage, &for_general.token),
        space,
        general,
        &item,
    );
    assert_eq!(answer, Some(Answer::Stored), "on a token for general");
}

/// With `general`'s wrap and entry of epoch 1 kept, the bot's token reads both while alice, the
/// bot's account, is a member who reads the history from before she joined. Once revisions drop
/// her, add her back at epoch 1 and drop her again, it reads neither, as an account without
/// `history` that joins after epoch 1.
#[test]
fn a_token_reads_its_channels_history_as_its_devices_account() {
    let (mut stage, mut laptop, _bot) = formed(3_545_700_021);
    let general = stage.f.general;
    let alice = stage.f.team.alice.account;
    let set_alice = |stage: &mut Stage, listed: bool| {
        stage.f.revise(|body| {
            let channel = body.channels.iter_mut().find(|c| c.id == general);
            let members = &mut channel.expect("general").members;
            members.retain(|member| member.account != alice);
            if listed {
                members.push(ChannelMember {
                    account: alice,
                    history: true,
                });
            }
        });
    };
    set_alice(&mut stage, true);
    let (alice_person, f) = (&stage.f.team.alice, &stage.f);
    let wrap_1 = wrap(general, 1, alice_person, alice_person);
    stores(&mut laptop, f, &wrap_1, "the wrap of epoch 1");
    stores(&mut laptop, f, &entry(general, 1), "the entry of epoch 1");
    let created = token(&stage, &[RECEIVE]);
    let read = |stage: &Stage| {
        let session = alone(stage, &created.token);
        [WRAPS, ENTRIES].map(|kind| given(&session, &stage.f, kind).len())
    };
    assert_eq!(read(&stage), [1, 1], "alice a member with history");

    for listed in [false, true, false] {
        set_alice(&mut stage, listed);
    }
    assert_eq!(read(&stage), [0, 0], "alice no member, added at epoch 1");
}

/// The bot's WebSocket on a token whose grant expires 8 seconds after it is signed closes at its
/// expiry; its WebSocket on a lasting token stays open and gets alice's next envelope.
#[test]
fn a_tokens_expiry_ends_its_own_connections_alone() {
    let stage = Stage::new(3_545_700_031);
    let lasting = token(&stage, &[SEND, RECEIVE]);
    let expires = now() + 8;
    let soon = token_for(&stage, &[stage.f.general], expires);
    let mut kept = link(&stage, &lasting.token);
    kept.sync(0);
    assert_eq!(kept.catch_up(), Vec::new(), "the lasting token's catch-up");
    let mut ending = socket(&stage, &soon.token);
    assert_open(&mut ending);

    while now() <= expires {
        thread::sleep(Duration::from_millis(100));
    }
    let since = Instant::now();
    assert_closes_within(&mut ending, since, END, "the expired token's WebSocket");
    let sent = to_bot(&stage, 1);
    assert_eq!(
        kept.entry(),
        messages(1, &sent).remove(0),
        "the lasting token's WebSocket"
    );
}
