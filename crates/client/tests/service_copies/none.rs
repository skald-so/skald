//! No copy goes to a service agent without a valid grant for the channel, a live token that
//! receives, a valid certificate, or while it is in the channel's `services`. Each test first sees
//! a copy reach the agent, or the agent's mailbox later hold only what it should, so that it fails
//! while no copy is sent at all.
//!
//! What the sending device checks itself, the grant, the roster and the certificate, it sends no
//! copy for. What only the server knows, whether a token lives and receives, the server's answers
//! decide: it stores no copy.

use serde_json::json;
use skald_proto::{Cbor as _, ChannelId, DeviceId, Grant, GrantBody, NonEmpty, Role, Unknown};

use crate::agents::{KEEP_EARLIER, PERMISSION, RECEIVE, SEND, id_of, own_record};
use crate::fixtures::send;
use crate::kit::{b64, latest, received};
use crate::revocations::revoked_by;
use crate::scene::{Scene, a2a};
use crate::world::{Person, next_by, next_with, now, run, set_role, store};

/// The space's grants as the server gives them to its members.
fn grants_request(scene: &Scene) -> String {
    format!("GET /spaces/{}/service-grants", scene.space())
}

/// The grant `signer`'s device signs for `service` in `channels` until `expires`.
fn grant(signer: &Person, service: DeviceId, channels: &[ChannelId], expires: u64) -> Grant {
    let body = GrantBody {
        service,
        channels: NonEmpty::from_vec(channels.to_vec()).expect("a channel"),
        signer: signer.device,
        expires,
        unknown: Unknown::default(),
    };
    let signature = signer.key.sign("skald grant", &[&body.to_cbor()]);
    Grant { body, signature }
}

/// `ci/deploy-bot`'s grant lists only the default channel: Ada's message in the talk's channel
/// sends it no copy; her next one in the default channel gives it one.
#[test]
fn no_copy_in_a_channel_the_grant_does_not_list() {
    let mut scene = Scene::with(
        [3_155_530_001, 3_155_530_002],
        ["ada-unlisted-standin", "bob-unlisted-standin"],
        Role::Member,
        |talk| vec![talk.space.default],
        &[],
    );
    let (channel, default) = (scene.talk.channel, scene.talk.space.default);

    let mark = scene.mark();
    scene.send_all(channel, "not for the bot's channels");
    scene.assert_none_sent(mark, "a channel the grant does not list");

    scene.send_all(default, "for the bot's channel");
    let copies = scene.copies();
    scene.assert_copies(&copies, default, &[("for the bot's channel", 1, None)]);
}

/// After a copy reaches `ci/deploy-bot`, the relay gives Ada's machine, as the space's grants, the
/// agent's grant for the channel signed by her device but expired a minute ago, though its token
/// lives: her next message sends it no copy.
#[test]
fn no_copy_with_an_expired_grant() {
    let mut scene = Scene::new(
        [3_155_530_011, 3_155_530_012],
        ["ada-expired-grant-standin", "bob-expired-grant-standin"],
    );
    let channel = scene.talk.channel;
    scene.send_all(channel, "before the grant expires");
    let copies = scene.copies();
    scene.assert_copies(&copies, channel, &[("before the grant expires", 1, None)]);

    let expired = grant(&scene.talk.ada, scene.bot(), &[channel], now() - 60);
    let answer = json!({ "grants": [b64(&expired.to_cbor())] }).to_string();
    (scene.cloud().relay).rewrite(&grants_request(&scene), move |_| answer.clone());
    let mark = scene.mark();
    scene.send_all(channel, "after the grant expired");

    scene.assert_none_sent(mark, "an expired grant");
}

/// Bob is an admin too. After a copy reaches `ci/deploy-bot`, whose grant Ada's device signed, a
/// revision Bob's device signs makes Ada a member, and her machine accepts it: her next message
/// sends the agent no copy, though its token lives.
#[test]
fn no_copy_once_the_grant_s_signer_is_no_longer_an_admin() {
    let mut scene = Scene::with(
        [3_155_530_021, 3_155_530_022],
        ["ada-demoted-standin", "bob-demoting-standin"],
        Role::Admin,
        |talk| vec![talk.channel],
        &[],
    );
    let channel = scene.talk.channel;
    scene.send_all(channel, "while ada is an admin");
    let copies = scene.copies();
    scene.assert_copies(&copies, channel, &[("while ada is an admin", 1, None)]);

    let (cloud, space) = (scene.cloud(), scene.space());
    let bob = &scene.talk.played[0].person;
    let account = scene.talk.ada.account;
    let demoted = next_by(&latest(cloud, &bob.token, space), bob, |members| {
        set_role(members, account, Role::Member);
    });
    store(cloud, &bob.token, &demoted);
    received(&scene.talk.ada.machine);
    let mark = scene.mark();
    scene.send_all(channel, "once ada is a member");

    scene.assert_none_sent(mark, "a grant whose signer is no admin");
}

/// `ci/deploy-bot` is created with a token that only sends: Ada's message gives it no stored copy.
/// A new token that receives replaces it, and the next message reaches the agent. Once that token
/// is revoked, and once another one's expiry is moved back in the database, the server stores no
/// copy of her messages; a last token that receives then finds only the copy of her last message
/// in the mailbox.
#[test]
fn no_copy_is_stored_without_a_live_token_that_receives() {
    let mut scene = Scene::with(
        [3_155_530_031, 3_155_530_032],
        ["ada-tokens-standin", "bob-tokens-standin"],
        Role::Member,
        |talk| vec![talk.channel],
        &[PERMISSION, SEND],
    );
    let channel = scene.talk.channel;
    let mark = scene.mark();
    scene.send_all(channel, "to a token that only sends");
    scene.assert_none_stored(mark, "a token that only sends");

    let receiving = scene.new_token(&[channel], &[PERMISSION, RECEIVE]);
    scene.send_all(channel, "to a token that receives");
    let copies = scene.copies_with(&receiving);
    let texts: Vec<String> = copies.iter().map(|copy| a2a(&copy.content).1).collect();
    assert_eq!(texts, ["to a token that receives"], "the copies");

    scene.revoke(&receiving);
    let mark = scene.mark();
    scene.send_all(channel, "to a revoked token");
    scene.assert_none_stored(mark, "a revoked token");

    let expiring = scene.new_token(&[channel], &[PERMISSION, RECEIVE, KEEP_EARLIER]);
    let id = id_of(&own_record(scene.cloud(), &expiring).expect("a live token"));
    scene.cloud().execute(&format!(
        "UPDATE service_tokens SET expires_at = now() - interval '1 second' WHERE id = '{id}'"
    ));
    let mark = scene.mark();
    scene.send_all(channel, "to an expired token");
    scene.assert_none_stored(mark, "an expired token");

    let last = scene.new_token(&[channel], &[PERMISSION, RECEIVE, KEEP_EARLIER]);
    scene.send_all(channel, "to the last token");
    let copies = scene.copies_with(&last);
    let texts: Vec<String> = copies.iter().map(|copy| a2a(&copy.content).1).collect();
    assert_eq!(texts, ["to the last token"], "the copies in the mailbox");
}

/// After a copy reaches `ci/deploy-bot`, Ada's device revokes the agent's device, and her machine
/// takes in what changed: her next message sends the agent no copy.
#[test]
fn no_copy_to_a_revoked_service_device() {
    let mut scene = Scene::new(
        [3_155_530_041, 3_155_530_042],
        ["ada-revokes-bot-standin", "bob-revoked-bot-standin"],
    );
    let channel = scene.talk.channel;
    scene.send_all(channel, "before the revocation");
    let copies = scene.copies();
    scene.assert_copies(&copies, channel, &[("before the revocation", 1, None)]);

    revoked_by(scene.cloud(), &scene.talk.ada, scene.bot());
    received(&scene.talk.ada.machine);
    let mark = scene.mark();
    scene.send_all(channel, "after the revocation");

    scene.assert_none_sent(mark, "a revoked device");
}

/// After a copy reaches `ci/deploy-bot`, a revision Ada's device signs lists its device in the
/// talk's channel's `services`, as a member agent that has not joined the group yet, and her
/// machine takes it in: her next message there sends it no copy.
#[test]
fn no_copy_to_an_agent_in_the_channel_s_services() {
    let mut scene = Scene::new(
        [3_155_530_051, 3_155_530_052],
        ["ada-services-standin", "bob-services-standin"],
    );
    let channel = scene.talk.channel;
    scene.send_all(channel, "before the agent is a member");
    let copies = scene.copies();
    scene.assert_copies(
        &copies,
        channel,
        &[("before the agent is a member", 1, None)],
    );

    let (cloud, space, bot) = (scene.cloud(), scene.space(), scene.bot());
    let ada = &scene.talk.ada;
    let listed = next_with(
        &latest(cloud, &ada.token, space),
        (ada.device, &ada.key),
        |body| {
            let entry = body
                .channels
                .iter_mut()
                .find(|entry| entry.id == channel)
                .expect("the talk's channel");
            entry.services.push(bot);
        },
    );
    store(cloud, &ada.token, &listed);
    // Ada's machine takes in the revision; whether it can bring the group in line is not this
    // test's matter.
    run(&ada.machine, &["recv"]);
    let mark = scene.mark();
    let _ = send(
        &scene.talk.ada.machine,
        channel,
        "once the agent is a member",
    );

    scene.assert_none_sent(mark, "an agent in the channel's services");
}
