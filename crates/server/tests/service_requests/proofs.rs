//! A token's requests that the server does not take, each reading, acking and storing nothing:
//! without its device's proof, with another device's proof, and once the token has ended. After
//! each case's refusals, the bot's mailbox still holds alice's envelopes, and a live token with the
//! bot's proof reads them.

use std::thread;
use std::time::Duration;

use skald_proto::{DeviceKind, PROTOCOL_VERSION};

use crate::bot::{
    assert_not_taken, challenge, challenge_response, https, server, sse, to_alice, to_bot, token,
};
use crate::common::device_flow::text;
use crate::common::devices::Device;
use crate::https::{AUTH_REQUIRED, DEVICE_REFUSED, Https, assert_problem, proof};
use crate::kit::assert_device_refused;
use crate::link::{messages, seqs};
use crate::tokens::{RECEIVE, SEND, Stage, grant};
use crate::wire::{Socket, assert_nowhere, now};

/// Checks that the bot's mailbox still holds its two entries.
#[track_caller]
fn assert_kept(stage: &Stage, what: &str) {
    assert_eq!(seqs(stage.db(), stage.bot.id), vec![1, 2], "{what}: acked");
}

/// Without a proof, the bot's long-poll, ack, send and stream with its live token are refused with
/// `401` and `auth-required`; with the bot's proof, the long-poll gives alice's envelopes.
#[test]
fn a_token_without_its_devices_proof_reads_acks_and_sends_nothing() {
    let stage = Stage::new(3_545_200_001);
    let created = token(&stage, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);
    let https = https(&stage, &created.token);

    let response = https.poll_request(0, 0, None).call();
    assert_problem(&response, 401, Some(AUTH_REQUIRED));
    let response = https.ack_request(2, None).call();
    assert_problem(&response, 401, Some(AUTH_REQUIRED));
    let (_, envelope) = to_alice(&stage, stage.f.general);
    let response = https.send_request(&envelope, false, None).call();
    assert_problem(&response, 401, Some(AUTH_REQUIRED));
    assert_nowhere(stage.db(), &envelope);
    let response = sse(&stage, &created.token).refused(Some(0), None);
    assert_problem(&response, 401, Some(AUTH_REQUIRED));
    assert_kept(&stage, "without a proof");

    assert_eq!(
        https.poll_now(0).entries,
        messages(1, &sent),
        "with the proof"
    );
}

/// The bot's token with the proof of alice's laptop, or of her other `service` device, on the
/// token's challenge: its long-poll, ack, send and stream are refused with `401` and
/// `device-refused`, and its WebSocket is closed with `device-refused`. alice's session with the
/// bot's proof is refused too. With the bot's proof, the token's long-poll gives alice's envelopes.
#[test]
fn a_token_with_another_devices_proof_reads_acks_and_sends_nothing() {
    let mut stage = Stage::new(3_545_200_011);
    let other = {
        let alice = &stage.f.team.alice;
        alice.certified_by(stage.db(), &alice.laptop, DeviceKind::Service, "Other")
    };
    stage.list_agent(&other, "other-bot");
    let created = token(&stage, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);
    let https = https(&stage, &created.token);
    let sse = sse(&stage, &created.token);
    let alice = &stage.f.team.alice;
    let flow = &stage.f.team.flow;

    let cases: [(&str, &Device); 2] = [("alice's laptop", &alice.laptop), ("other", &other)];
    for (what, device) in cases {
        let by = |https: &Https| {
            proof(
                &https.challenge(),
                device.id,
                &device.signing.key,
                https.public(),
            )
        };
        let response = https.poll_request(0, 0, Some(by(&https))).call();
        assert_problem(&response, 401, Some(DEVICE_REFUSED));
        let response = https.ack_request(2, Some(by(&https))).call();
        assert_problem(&response, 401, Some(DEVICE_REFUSED));
        let (_, envelope) = to_alice(&stage, stage.f.general);
        let response = https
            .send_request(&envelope, false, Some(by(&https)))
            .call();
        assert_problem(&response, 401, Some(DEVICE_REFUSED));
        assert_nowhere(stage.db(), &envelope);
        let response = sse.refused(Some(0), Some(&by(sse.https())));
        assert_problem(&response, 401, Some(DEVICE_REFUSED));
        let mut socket = Socket::on_session(server(&stage), flow, &created.token, device);
        assert_device_refused(&mut socket);
        assert_kept(&stage, what);
    }

    let as_bot = Https::on_session(server(&stage), flow, alice.session.token(), &stage.bot);
    let response = as_bot.poll_request(0, 0, Some(as_bot.proof())).call();
    assert_problem(&response, 401, Some(DEVICE_REFUSED));
    assert_device_refused(&mut Socket::to(server(&stage), flow, alice, &stage.bot));
    assert_kept(&stage, "alice's session");

    assert_eq!(
        https.poll_now(0).entries,
        messages(1, &sent),
        "the bot's proof"
    );
}

/// Once the token has ended, as `end` ends it: a new challenge is refused with `401`, and the
/// challenges it got before serve no long-poll, ack, send, stream or WebSocket, each refused with
/// `401` or closed.
#[track_caller]
fn assert_ended_token_gets_nothing(stage: &Stage, token: &str, end: impl FnOnce()) {
    let https = https(stage, token);
    let sse = sse(stage, token);
    let earlier: Vec<Vec<u8>> = (0..5).map(|_| challenge(stage, token)).collect();
    end();

    let response = challenge_response(stage, token);
    assert_problem(&response, 401, None);
    let response = https
        .poll_request(0, 0, Some(https.proof_on(&earlier[0])))
        .call();
    assert_problem(&response, 401, None);
    let response = https
        .ack_request(2, Some(https.proof_on(&earlier[1])))
        .call();
    assert_problem(&response, 401, None);
    let (_, envelope) = to_alice(stage, stage.f.general);
    let response = https
        .send_request(&envelope, false, Some(https.proof_on(&earlier[2])))
        .call();
    assert_problem(&response, 401, None);
    assert_nowhere(stage.db(), &envelope);
    let response = sse.refused(Some(0), Some(&sse.https().proof_on(&earlier[3])));
    assert_problem(&response, 401, None);
    let flow = &stage.f.team.flow;
    let mut socket = Socket::on_challenge(
        server(stage),
        flow,
        &earlier[4],
        &stage.bot,
        PROTOCOL_VERSION,
    );
    assert_not_taken(&mut socket, "a WebSocket");
    assert_kept(stage, "an ended token");
}

/// alice's session revokes the bot's token: the token gets nothing; another live token of the bot
/// still reads alice's envelopes.
#[test]
fn a_revoked_token_reads_acks_and_sends_nothing() {
    let stage = Stage::new(3_545_200_021);
    let revoked = token(&stage, &[SEND, RECEIVE]);
    let kept = token(&stage, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);

    assert_ended_token_gets_nothing(&stage, &revoked.token, || {
        let response = stage.revoke_as(&stage.f.team.alice, &revoked.id);
        assert_eq!(response.status, 204, "revoked: {}", text(&response));
    });
    let https = https(&stage, &kept.token);
    assert_eq!(
        https.poll_now(0).entries,
        messages(1, &sent),
        "another token"
    );
}

/// The bot's token whose grant expires 8 seconds after it is signed gets nothing once it has
/// passed; a token of a grant for a day still reads alice's envelopes.
#[test]
fn an_expired_token_reads_acks_and_sends_nothing() {
    let stage = Stage::new(3_545_200_031);
    let alice = &stage.f.team.alice;
    let lasting = token(&stage, &[SEND, RECEIVE]);
    let expires = now() + 8;
    let soon = grant(stage.bot.id, &[stage.f.general], &alice.laptop, expires);
    let soon = stage.create_as(alice, stage.bot.id, &soon, &[SEND, RECEIVE]);
    let sent = to_bot(&stage, 2);

    assert_ended_token_gets_nothing(&stage, &soon.token, || {
        while now() <= expires {
            thread::sleep(Duration::from_millis(200));
        }
    });
    let https = https(&stage, &lasting.token);
    assert_eq!(
        https.poll_now(0).entries,
        messages(1, &sent),
        "a lasting token"
    );
}
