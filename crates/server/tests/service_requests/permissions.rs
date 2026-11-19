//! A token's permissions with its device's proof: reads and acks need the receive permission;
//! sends need the send permission and a channel of the token's grant. A refused request reads,
//! acks and stores nothing.

use crate::api::assert_refused;
use crate::bot::{
    assert_no_entry, assert_send_refused, assert_sends, assert_socket_send_refused, https, ops,
    socket, sse, to_alice, to_bot, token,
};
use crate::link::{messages, seqs};
use crate::tokens::{RECEIVE, SEND, Stage};
use crate::wire::{Answer, NOT_A_MEMBER, assert_nowhere, envelopes_of};

/// A send-only token of the bot, with the bot's proof: its long-poll, ack and stream are refused,
/// and on its WebSocket a `sync` and an `ack` give and delete nothing, so the bot's mailbox still
/// holds alice's two envelopes, which a receiving token reads. It still sends.
#[test]
fn a_token_without_the_receive_permission_reads_and_acks_nothing() {
    let stage = Stage::new(3_545_300_001);
    let receiving = token(&stage, &[SEND, RECEIVE]);
    let send_only = token(&stage, &[SEND]);
    let sent = to_bot(&stage, 2);
    let https = https(&stage, &send_only.token);

    assert_sends(&stage, &https);

    let response = https.poll_request(0, 0, Some(https.proof())).call();
    assert_refused(&response, "a long-poll");
    let response = https.ack_request(2, Some(https.proof())).call();
    assert_refused(&response, "an ack");
    let sse = sse(&stage, &send_only.token);
    let response = sse.refused(Some(0), Some(&sse.https().proof()));
    assert_refused(&response, "a stream");
    let mut socket = socket(&stage, &send_only.token);
    socket.sync(0);
    socket.ack(2);
    assert_no_entry(&mut socket, "a WebSocket");
    assert_eq!(seqs(stage.db(), stage.bot.id), vec![1, 2], "acked");

    let receiving = crate::bot::https(&stage, &receiving.token);
    assert_eq!(receiving.poll_now(0).entries, messages(1, &sent));
}

/// A receive-only token of the bot, with the bot's proof: its send to alice's laptop is refused
/// over HTTPS and on its WebSocket, and nothing of either is stored; it still reads alice's
/// envelope.
#[test]
fn a_token_without_the_send_permission_sends_nothing() {
    let stage = Stage::new(3_545_300_011);
    let receive_only = token(&stage, &[RECEIVE]);
    let sent = to_bot(&stage, 1);
    let https = https(&stage, &receive_only.token);
    assert_eq!(https.poll_now(0).entries, messages(1, &sent), "it reads");

    let (_, envelope) = to_alice(&stage, stage.f.general);
    let response = https
        .send_request(&envelope, false, Some(https.proof()))
        .call();
    assert_send_refused(&stage, &response, &envelope, "over HTTPS");
    let mut socket = socket(&stage, &receive_only.token);
    let (_, envelope) = to_alice(&stage, stage.f.general);
    socket.send(&envelope);
    assert_socket_send_refused(&stage, &mut socket, &envelope, "on a WebSocket");
    let alice = stage.f.team.alice.laptop.id;
    assert_eq!(envelopes_of(stage.db(), alice), Vec::<Vec<u8>>::new());
}

/// The bot's token for `general`, with the bot's proof, sends to alice's laptop in `ops`, a channel
/// of alice and bob that its grant does not list: `send-refused` with `not-a-member` over HTTPS and
/// on its WebSocket, nothing stored. Its send in `general` is stored.
#[test]
fn a_token_sends_in_no_channel_outside_its_grant() {
    let mut stage = Stage::new(3_545_300_021);
    let ops = ops(&mut stage);
    let created = token(&stage, &[SEND, RECEIVE]);
    let https = https(&stage, &created.token);

    let (header, envelope) = to_alice(&stage, ops);
    let answer = https.send(&envelope, false);
    assert_eq!(
        answer,
        Answer::Refused(crate::wire::key_of(&header), NOT_A_MEMBER.to_owned()),
        "over HTTPS"
    );
    assert_nowhere(stage.db(), &envelope);
    let mut socket = socket(&stage, &created.token);
    let (header, envelope) = to_alice(&stage, ops);
    let (key, problem) = socket.refusal(header.msg_id, &envelope);
    assert_eq!(
        (key, problem.as_str()),
        (crate::wire::key_of(&header), NOT_A_MEMBER),
        "on a WebSocket"
    );
    assert_nowhere(stage.db(), &envelope);

    assert_sends(&stage, &https);
}
