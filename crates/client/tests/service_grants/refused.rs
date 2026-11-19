//! Content from a `service` device that fails a check, and content with a grant from a `client`
//! device, is dropped with a warning that names the sending device; its text is never shown.

use skald_proto::{Cbor as _, ChannelId, Envelope, RawEnvelope, Role, SpaceId};

use crate::fixtures::{Issuer, add_device};
use crate::played::{Played, mls_envelope};
use crate::talk::Line;
use crate::world::{HOST, now};
use crate::{Bot, GRANTED, Note, grant, service_device};

/// The one valid message among the dropped ones.
const VALID: &str = "deploy-bot: the canary is healthy";

/// Each dropped message's text; none is part of another or of [`VALID`].
const OTHER_AGENT: &str = "sent as another agent of the owner ci";
const AS_PERSON: &str = "sent as ada herself";
const AS_ADAS_AGENT: &str = "sent as an agent of ada's account";
const UNGRANTED: &str = "no grant, with a counter";
const BARE: &str = "no grant and no counter";
const FOR_ANOTHER: &str = "a grant that names another service device";
const OTHER_CHANNEL: &str = "a grant for the default channel only";
const BY_MEMBER: &str = "a grant bob signed, a member who is no admin";
const BY_FORGED: &str = "a grant by a device only the server certified";
const BY_SERVICE: &str = "a grant by a service device";
const BROKEN: &str = "a grant whose signature is broken";
const EXPIRED: &str = "a grant that expired a minute ago";
const UNLISTED: &str = "from a service device no revision lists";
const CLIENT_HPKE: &str = "bob's client device with a grant, sealed";
const CLIENT_MLS: &str = "bob's client device with a grant, in the group";

/// In Ada's catch-up, beside a valid message of the service agent, envelopes a compromised server
/// or member could put there: from the agent's device, with `from` another agent of its owner, Ada's
/// person address or an agent of Ada's account; without a grant, with or without `counter`; with a
/// grant that names another service device, lists only the space's default channel, is signed by
/// Bob's device, a member's, by a `client` device only the server certified or by a `service`
/// device, has a broken signature or has expired; from a `service` device of Ada's account that no
/// revision lists, with a valid grant for it; from Bob's `client` device with a grant for it, sealed
/// to Ada's device and encrypted in the channel's group. Ada's `skald recv` shows the valid one
/// once and drops the others, each with a warning naming it and its sending device.
#[test]
fn content_failing_a_grant_or_entry_check_is_dropped_with_a_warning() {
    let mut bot = Bot::new(
        [4_235_021_001, 4_235_021_002],
        ["ada-checks-grants-standin", "bob-member-grants-standin"],
        Role::Member,
        true,
    );
    let cloud = bot.cloud();
    let (ada, bob) = (&bot.talk.ada, &bot.talk.played[0].person);
    let unlisted = service_device(cloud, ada, "stray-bot-runner");
    let forged = add_device(cloud, ada.account, "ada-forged", &Issuer::Server);
    // Ada's machine takes in its account's new devices before the test.
    crate::kit::received(&ada.machine);
    let (channel, expires) = (bot.talk.channel, now() + GRANTED);
    let by_ada = (ada.device, &ada.key);
    let space = bot.talk.space.name.clone();
    let granted = || Some(bot.granted());
    let adas_agent = format!("{space}/{}/claude-api@{HOST}", ada.handle);
    let to_bot = |signer, channel| Some(grant(signer, bot.device.id, &[channel], expires));
    let mut broken = bot.granted();
    broken.signature[0] ^= 1;
    let expired = grant(by_ada, bot.device.id, &[channel], now() - 60);
    let mut from_bot = vec![
        Note::new(
            &format!("{space}/ci/release-bot@{HOST}"),
            granted(),
            OTHER_AGENT,
        ),
        Note::new(&bot.talk.address(ada), granted(), AS_PERSON),
        Note::new(&adas_agent, granted(), AS_ADAS_AGENT),
        Note::new(&bot.address, None, BARE),
        Note::new(
            &bot.address,
            Some(grant(by_ada, unlisted.id, &[channel], expires)),
            FOR_ANOTHER,
        ),
        Note::new(
            &bot.address,
            to_bot(by_ada, bot.talk.space.default),
            OTHER_CHANNEL,
        ),
        Note::new(
            &bot.address,
            to_bot((bob.device, &bob.key), channel),
            BY_MEMBER,
        ),
        Note::new(
            &bot.address,
            to_bot((forged.id, &forged.signing), channel),
            BY_FORGED,
        ),
        Note::new(
            &bot.address,
            to_bot((unlisted.id, &unlisted.signing), channel),
            BY_SERVICE,
        ),
        Note::new(&bot.address, Some(broken), BROKEN),
        Note::new(&bot.address, Some(expired), EXPIRED),
    ];
    let mut ungranted = Note::new(&bot.address, None, UNGRANTED);
    ungranted.counter = Some(1);
    from_bot.push(ungranted);
    let mut envelopes = vec![bot.sealed(&bot.note(VALID))];
    envelopes.extend(from_bot.iter().map(|note| bot.sealed(note)));

    let unlisted_grant = grant(by_ada, unlisted.id, &[channel], expires);
    let unlisted_from = format!("{space}/ci/stray-bot@{HOST}");
    let from_unlisted = Note::new(&unlisted_from, Some(unlisted_grant), UNLISTED);
    envelopes.push(bot.sealed_by(&from_unlisted, &unlisted.signing));
    let bob_grant = grant(by_ada, bob.device, &[channel], expires);
    let mut client_hpke = Note::new(&bot.talk.address(bob), Some(bob_grant.clone()), CLIENT_HPKE);
    client_hpke.counter = Some(1);
    envelopes.push(bot.sealed_by(&client_hpke, &bob.key));
    let client_mls = Note::new(&bot.talk.address(bob), Some(bob_grant), CLIENT_MLS);
    let (bob_device, at) = (bob.device, bot.talk.at());
    envelopes.push(in_group(&mut bot.talk.played[0], at, &client_mls));

    let finished = bot.ada_receives(envelopes);

    bot.assert_shown(&finished, VALID);
    for note in &from_bot {
        bot.assert_dropped(&finished, note, bot.device.id);
    }
    bot.assert_dropped(&finished, &from_unlisted, unlisted.id);
    bot.assert_dropped(&finished, &client_hpke, bob_device);
    bot.assert_dropped(&finished, &client_mls, bob_device);
}

/// `note`, with `counter` 1, as the `mls` envelope of `at`'s channel that `played` encrypts in the
/// channel's group.
fn in_group(played: &mut Played, at: (SpaceId, ChannelId), note: &Note) -> RawEnvelope {
    let mut header = mls_envelope(at, &[], &[]).envelope().header.clone();
    header.msg_id = note.msg_id;
    let mut content = Line::new(&note.from, 1, note.text).content(note.msg_id);
    content.grant = note.grant.clone();
    let payload = (played.mls)
        .encrypt(at.1, &content.to_cbor())
        .unwrap_or_else(|err| panic!("encrypt in the channel's group: {err:?}"));
    RawEnvelope::from_bytes(Envelope { header, payload }.to_cbor()).expect("an envelope")
}
