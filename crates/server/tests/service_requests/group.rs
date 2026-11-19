//! The bot as a member of `general`'s MLS group, as a `service` device that keeps state: its
//! Commit needs a token with the send permission and the bot's proof; its GroupInfo goes up with
//! the token alone.

use skald_crypto::{MlsDevice, SigningKey, StateKey};
use skald_proto::{Cbor, Cert};

use crate::bot::{alone, assert_socket_send_refused, socket, token};
use crate::scene::{Peer, assert_latest, assert_uploaded, mls_envelope, mls_header, upload};
use crate::tokens::{RECEIVE, SEND, Stage};
use crate::wire::{Answer, now, send_frame};

/// `stage` with `general`'s group formed by alice's laptop with the bot, which joined from the
/// Welcome, and the bot in the channel's `services`: alice's laptop and the bot's MLS device.
pub fn formed(github: u64) -> (Stage, Peer, MlsDevice) {
    let mut stage = Stage::new(github);
    let general = stage.f.general;
    let bot_id = stage.bot.id;
    stage.f.revise(|body| {
        let entry = body.channels.iter_mut().find(|c| c.id == general);
        entry.expect("general").services.push(bot_id);
    });
    let alice = &stage.f.team.alice;
    let certs = alice.session.devices(stage.space());
    let bot_cert = certs[&bot_id.to_string()]
        .clone()
        .expect("the bot's certificate");
    let admitted =
        [&alice.cert, &bot_cert].map(|cert| Cert::from_cbor(cert).expect("a certificate"));
    let signing = SigningKey::from_bytes(&stage.bot.signing.secret);
    let mut bot = MlsDevice::new(&signing, &StateKey::generate());
    let package = bot.key_package(now()).expect("a KeyPackage");

    let mut laptop = Peer::of(&stage.f, alice);
    let commit = laptop.creates(&stage.f, &[package.message], &admitted);
    laptop.stores(&stage.f, &commit, &[]);
    let (welcome, _) = laptop.merges_and_uploads(&stage.f);
    let welcome = welcome.expect("a Welcome");
    let joined = bot
        .join(&welcome, now(), |_| true)
        .expect("joins from the Welcome");
    assert_eq!(joined, general, "the channel joined");
    (stage, laptop, bot)
}

/// The bot's empty Commit on a WebSocket of a receive-only token is refused and nothing of it is
/// stored. The same Commit, in an envelope of its own, on a WebSocket of a send-only token is
/// stored; the token alone then uploads the bot's GroupInfo of the new epoch, which alice's session
/// gets.
#[test]
fn a_commit_needs_the_send_permission_and_its_group_info_goes_up_with_the_token_alone() {
    let (stage, _laptop, mut bot) = formed(3_545_400_001);
    let general = stage.f.general;
    let receive_only = token(&stage, &[RECEIVE]);
    let send_only = token(&stage, &[SEND]);
    let commit = bot
        .commit_empty(general, b"", now())
        .expect("an empty Commit");

    let refused = mls_envelope(&mls_header(&stage.f, &[]), &commit);
    let mut socket_r = socket(&stage, &receive_only.token);
    socket_r.frame(send_frame(&refused, false));
    assert_socket_send_refused(&stage, &mut socket_r, &refused, "a receive-only token");

    let stored = mls_envelope(&mls_header(&stage.f, &[]), &commit);
    let mut socket_s = socket(&stage, &send_only.token);
    socket_s.frame(send_frame(&stored, false));
    let answer = socket_s.answer();
    assert!(
        matches!(answer, Answer::Stored(_)),
        "a send-only token's Commit stored, not {answer:?}"
    );
    bot.commit_stored(general)
        .expect("the stored Commit merges");

    let group_info = bot.group_info(general).expect("the bot's GroupInfo");
    let response = upload(&alone(&stage, &send_only.token), &stage.f, &group_info);
    assert_uploaded(&response, "the bot's");
    let alice = &stage.f.team.alice;
    assert_latest(
        &alice.session,
        &stage.f,
        &group_info,
        "after the bot's Commit",
    );
}
