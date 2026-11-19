//! A token with its device's proof connects over WebSocket, long-poll and SSE, reads and acks the
//! bot's mailbox and sends in its channel.

use crate::bot::{assert_alice_holds, assert_sends, https, link, sse, to_alice, to_bot, token};
use crate::https::batch_of;
use crate::link::{Entry, PUSH, await_seqs, messages};
use crate::tokens::{RECEIVE, SEND, Stage};

/// The bot's WebSocket on its token's challenge, its `auth` signed by the bot: catch-up gives
/// alice's two envelopes, its ack deletes them, the next one is pushed, and its envelope to alice's
/// laptop in `general` is stored.
#[test]
fn a_token_with_its_devices_proof_reads_acks_and_sends_over_a_websocket() {
    let stage = Stage::new(3_545_100_001);
    let created = token(&stage, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);
    let bot = stage.bot.id;

    let mut link = link(&stage, &created.token);
    link.sync(0);
    assert_eq!(link.catch_up(), messages(1, &sent), "catch-up");
    link.ack(2);
    await_seqs(stage.db(), bot, &[]);
    let next = to_bot(&stage, 1);
    assert_eq!(link.entry(), Entry::Message(3, next[0].clone()), "pushed");

    let (header, envelope) = to_alice(&stage, stage.f.general);
    link.store(&header, &envelope, false);
    assert_alice_holds(&stage, &envelope);
}

/// The bot's long-polls with its token and proof: one with a wait of 0 gives alice's two
/// envelopes, an ack deletes them, a waiting one answers the next, and its send to alice's laptop
/// is stored.
#[test]
fn a_token_with_its_devices_proof_long_polls_acks_and_sends_over_https() {
    let stage = Stage::new(3_545_100_011);
    let created = token(&stage, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);
    let bot = stage.bot.id;
    let https = https(&stage, &created.token);

    assert_eq!(https.poll_now(0).entries, messages(1, &sent), "a long-poll");
    https.ack(2);
    await_seqs(stage.db(), bot, &[]);
    let waiting = https.start_poll(2, crate::https::LONG);
    let next = to_bot(&stage, 1);
    let (response, _) = waiting.answer();
    assert_eq!(
        batch_of(&response).entries,
        messages(3, &next),
        "a waiting long-poll"
    );

    assert_sends(&stage, &https);
}

/// The bot's stream with its token and proof: catch-up gives alice's envelope, the next one comes
/// on it, and an ack over HTTPS with the same token deletes both.
#[test]
fn a_token_with_its_devices_proof_streams_its_mailbox_over_sse() {
    let stage = Stage::new(3_545_100_021);
    let created = token(&stage, &[SEND, RECEIVE]);
    let first = to_bot(&stage, 1);
    let sse = sse(&stage, &created.token);

    let mut stream = sse.open(Some(0));
    assert_eq!(stream.catch_up(), messages(1, &first), "catch-up");
    let next = to_bot(&stage, 1);
    assert_eq!(
        stream.entry_within(PUSH),
        Entry::Message(2, next[0].clone()),
        "pushed"
    );
    sse.https().ack(2);
    await_seqs(stage.db(), stage.bot.id, &[]);
}
