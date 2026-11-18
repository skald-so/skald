//! The checks of content from a `service` device by the latest revision and device certificates
//! the client has: a revision it has not accepted yet that lists the agent, a revision that makes
//! the grant's signer a member, the device's revocation, and a certificate of the device on an
//! account outside the space.

use skald_crypto::issue;
use skald_proto::{CertBody, DeviceKind, NonEmpty, Role, Unknown};

use crate::fixtures::cert_of;
use crate::kit::{latest, rosters};
use crate::revocations::revoked_by;
use crate::world::{change_devices, device_entry, next_with, now, set_role, stale_once, store};
use crate::{BOT_HOST, Bot, GRANTED, Note, grant};

/// What the service agent says; none is anywhere else.
const FIRST: &str = "deploy-bot: the first build of the day is green";
const BEFORE: &str = "deploy-bot: bob approved this rollout";
const AFTER: &str = "deploy-bot: bob approved this rollback too";
const STILL: &str = "deploy-bot: ada approved the hotfix";
const LATER: &str = "deploy-bot: the nightly build is green";

/// A revision Ada's machine has not accepted yet lists the service agent, and the first answer to a
/// request for the space's revisions leaves it out, as one the server gave before it stored that
/// revision. Ada's `skald recv`, finding no entry for the agent's device, fetches the revisions
/// again, accepts it and shows the agent's message.
#[test]
fn content_from_an_agent_a_revision_not_yet_accepted_lists_is_shown() {
    let bot = Bot::new(
        [4_235_031_001, 4_235_031_002],
        ["ada-lists-a-bot-standin", "bob-sees-the-bot-standin"],
        Role::Member,
        false,
    );
    let space = bot.talk.space.id;
    let held = rosters(bot.cloud(), &bot.talk.ada.token, space).len();
    stale_once(bot.cloud(), space, held - 1);

    let finished = bot.ada_receives(vec![bot.sealed(&bot.note(FIRST))]);

    bot.assert_shown(&finished, FIRST);
}

/// Bob, an admin, signs the agent's grant, and Ada's `skald recv` shows its message. A revision
/// Ada's device signs then makes Bob a member: the next message with Bob's grant is dropped with a
/// warning naming the agent's device, while one with Ada's grant is shown.
#[test]
fn grant_fails_once_a_later_revision_makes_its_signer_a_member() {
    let bot = Bot::new(
        [4_235_032_001, 4_235_032_002],
        ["ada-demotes-bob-standin", "bob-grants-once-standin"],
        Role::Admin,
        true,
    );
    let (ada, bob) = (&bot.talk.ada, &bot.talk.played[0].person);
    let expires = now() + GRANTED;
    let bobs = grant(
        (bob.device, &bob.key),
        bot.device.id,
        &[bot.talk.channel],
        expires,
    );
    let by_bob = |text| Note::new(&bot.address, Some(bobs.clone()), text);
    let first = bot.ada_receives(vec![bot.sealed(&by_bob(BEFORE))]);
    bot.assert_shown(&first, BEFORE);
    let previous = latest(bot.cloud(), &ada.token, bot.talk.space.id);
    let demoted = next_with(&previous, (ada.device, &ada.key), |body| {
        let mut members = body.members.clone().into_vec();
        set_role(&mut members, bob.account, Role::Member);
        body.members = NonEmpty::from_vec(members).expect("a member");
    });
    store(bot.cloud(), &ada.token, &demoted);

    let after = by_bob(AFTER);
    let next = bot.ada_receives(vec![bot.sealed(&after), bot.sealed(&bot.note(STILL))]);

    bot.assert_dropped(&next, &after, bot.device.id);
    bot.assert_shown(&next, STILL);
}

/// Ada's `skald recv` shows the agent's message; Ada's device then revokes the agent's device, and
/// its next message is dropped with a warning naming it.
#[test]
fn content_from_a_revoked_service_device_is_dropped() {
    let bot = Bot::new(
        [4_235_033_001, 4_235_033_002],
        ["ada-revokes-a-bot-standin", "bob-outlives-the-bot-standin"],
        Role::Member,
        true,
    );

    shown_then_dropped(&bot, || {
        revoked_by(bot.cloud(), &bot.talk.ada, bot.device.id);
    });
}

/// Ada's `skald recv` shows the agent's message. From then on, as a compromised server would, the
/// relay gives the agent's device a certificate on Dave's account, outside the space, issued by
/// Dave's device, with Dave's device's own: the agent's next message is dropped with a warning
/// naming its device.
#[test]
fn content_from_a_service_device_certified_on_an_account_outside_the_space_is_dropped() {
    let bot = Bot::new(
        [4_235_034_001, 4_235_034_002],
        ["ada-keeps-the-bot-standin", "bob-near-the-bot-standin"],
        Role::Member,
        true,
    );
    let dave = (bot.talk.world).sign_up(4_235_034_003, "dave-outside-standin", "dave-laptop");
    let body = CertBody {
        account: dave.account,
        device: bot.device.id,
        kind: DeviceKind::Service,
        signing: bot.device.signing.public_key(),
        hpke: bot.device.hpke.public_key(),
        issuer: Some(dave.device),
        issued: now(),
        ssh: None,
        name: BOT_HOST.to_owned(),
        unknown: Unknown::default(),
    };
    let on_dave = issue(&body, &dave.key, &bot.device.signing).expect("a certificate");
    let entries = [
        device_entry(bot.device.id, &on_dave),
        device_entry(dave.device, &cert_of(bot.cloud(), &dave)),
    ];
    let id = bot.device.id.to_string();

    shown_then_dropped(&bot, || {
        change_devices(bot.cloud(), bot.talk.space.id, move |devices| {
            devices.retain(|entry| entry["device"].as_str() != Some(id.as_str()));
            devices.extend(entries.iter().cloned());
        });
    });
}

/// Ada's `skald recv` shows the agent's message [`FIRST`]; after `change`, its next one is dropped
/// with a warning naming it and the agent's device.
#[track_caller]
fn shown_then_dropped(bot: &Bot, change: impl FnOnce()) {
    let first = bot.ada_receives(vec![bot.sealed(&bot.note(FIRST))]);
    bot.assert_shown(&first, FIRST);
    change();

    let later = bot.note(LATER);
    let next = bot.ada_receives(vec![bot.sealed(&later)]);

    bot.assert_dropped(&next, &later, bot.device.id);
}
