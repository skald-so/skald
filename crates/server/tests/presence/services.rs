//! alice's `service` device, the bot, in `general`'s group with alice's laptop: its value goes up,
//! and the channel's values come to it, only on a token with the `send` permission that lists
//! `general`, with the bot's proof.

use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{Cbor as _, Cert};

use crate::bot::{ops, token};
use crate::kit::{
    Caller, Channel, assert_all_kept, assert_given, assert_kept, assert_refused_with,
    assert_unproved,
};
use crate::scene::Peer;
use crate::tokens::{RECEIVE, SEND, Stage, grant};
use crate::wire::now;

/// The stage with the bot in `general`'s `services`, and `general`'s group formed by alice's
/// laptop with the bot, which joined from the Welcome; gives alice's laptop and the channel.
pub fn with_bot(github: u64) -> (Stage, Peer, Channel) {
    let mut stage = Stage::new(github);
    let (general, bot_id) = (stage.f.general, stage.bot.id);
    stage.f.revise(|body| {
        for channel in body.channels.iter_mut().filter(|c| c.id == general) {
            channel.services.push(bot_id);
        }
    });
    let mut bot = MlsDevice::new(
        &SigningKey::from_bytes(&stage.bot.signing.secret),
        &StateKey::generate(),
    );
    let alice = &stage.f.team.alice;
    let certs = alice.session.devices(stage.space());
    let of_bot = (certs.get(&bot_id.to_string()).cloned().flatten()).expect("the bot's cert");
    let admitted: Vec<Cert> = [alice.cert.as_slice(), of_bot.as_slice()]
        .iter()
        .map(|bytes| Cert::from_cbor(bytes).expect("a certificate"))
        .collect();
    let package = (bot.key_package(now()))
        .expect("the bot's KeyPackage")
        .message;

    let mut laptop = Peer::of(&stage.f, alice);
    let commit = laptop.creates(&stage.f, &[package], &admitted);
    laptop.stores(&stage.f, &commit, &[]);
    let welcome = (laptop.merges_and_uploads(&stage.f).0).expect("the Welcome");
    let joined = (bot.join(&welcome, now(), |_| true)).expect("the bot joins");
    assert_eq!(joined, general, "the bot's channel");
    let channel = Channel {
        space: stage.space(),
        id: general,
        epoch: (laptop.mls.epoch(general)).expect("the laptop's epoch"),
    };
    (stage, laptop, channel)
}

/// The bot's requests with `token` and its proof.
pub fn bot_caller(stage: &Stage, token: &str) -> Caller {
    let flow = &stage.f.team.flow;
    Caller::on(&flow.stand.server, flow, token, &stage.bot)
}

/// On a token with the send permission, the bot uploads its value, which alice's laptop gets with
/// its own, and gets both.
#[test]
fn a_service_member_device_uploads_and_reads_on_a_token_that_sends() {
    let (stage, laptop, general) = with_bot(3_621_500_001);
    let alice = Caller::of(&stage.f.team.flow, &stage.f.team.alice);
    let bot = bot_caller(&stage, &token(&stage, &[SEND]).token);
    let of_alice = general.value(laptop.id);
    let of_bot = general.value(stage.bot.id);
    assert_all_kept(&general, &[(&alice, &of_alice), (&bot, &of_bot)]);
    assert_given(&[&alice, &bot], &general, &[&of_alice, &of_bot], "both");
}

/// alice's laptop uploads a value. The bot gets nothing on a token without the send permission, on
/// one whose grant lists only `ops`, or on a sending token without its proof; on a token for
/// `general` with the send permission, it gets alice's value.
#[test]
fn a_token_without_send_or_the_channel_gets_nothing() {
    let (mut stage, laptop, general) = with_bot(3_621_500_011);
    let ops = ops(&mut stage);
    let alice = &stage.f.team.alice;
    let of_alice = general.value(laptop.id);
    let laptop = Caller::of(&stage.f.team.flow, alice);
    assert_kept(&laptop.put(&general, &of_alice), "alice's");

    let receive_only = token(&stage, &[RECEIVE]);
    let for_ops = grant(stage.bot.id, &[ops], &alice.laptop, now() + 86_400);
    let ops_only = stage.create_as(alice, stage.bot.id, &for_ops, &[SEND, RECEIVE]);
    for (created, what) in [(&receive_only, "receive only"), (&ops_only, "ops only")] {
        let response = bot_caller(&stage, &created.token).get(&general, true);
        assert_refused_with(&response, None, what);
    }

    let sending = bot_caller(&stage, &token(&stage, &[SEND]).token);
    assert_unproved(&sending.get(&general, false), "the token alone");
    assert_given(
        &[&sending],
        &general,
        &[&of_alice],
        "a token that sends in general",
    );
}
