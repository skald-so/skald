//! An admin removes a service agent: its tokens and runs are refused, the members' devices find no
//! entry, and its device's own account revokes it.

use skald_crypto::accept_next;

use crate::changes::{in_services, lists, remove_agent, revoked};
use crate::kit::latest;
use crate::member::Bot;
use crate::revocations::revoked_by_cli;
use crate::scene::Scene;
use crate::service_kit::{BOT, entry_of, own_record};
use crate::world::space_certs;

/// What the agent says; nowhere else.
const BEFORE_SAYS: &str = "removal test: deploy-bot reports before its removal";
const AFTER_SAYS: &str = "removal test: deploy-bot tries after its removal";

/// The stateless agent sends. Ada's machine removes it: the space's next revision, signed by Ada's
/// device, has no entry of the agent, and Bob's device accepts it. The agent's token is refused,
/// and its runs send and receive nothing. Ada's account then revokes the agent's device, with
/// `skald revoke-device` unless the removal did, and the server holds the revocation.
#[test]
fn after_an_admin_removes_the_agent_its_tokens_and_runs_are_refused() {
    let scene = Scene::new(
        [3_135_910_001, 3_135_910_002],
        ["ada-removal-standin", "bob-removal-standin"],
    );
    let (cloud, space, channel, ada) = (scene.cloud(), scene.space(), scene.channel(), scene.ada());
    scene.agent.sent(channel, BEFORE_SAYS);
    let before = latest(cloud, &ada.token, space);

    let removed = remove_agent(ada);

    assert!(removed.success, "{removed:?}");
    let bob = scene.bob();
    let after = latest(cloud, &bob.token, space);
    assert_eq!(
        after.body().revision,
        before.body().revision + 1,
        "{after:?}"
    );
    assert_eq!(after.body().signer, ada.device, "{after:?}");
    assert_eq!(entry_of(&after, BOT), None, "{after:?}");
    assert!(
        !lists(&after, scene.device),
        "the agent is still listed: {after:?}"
    );
    let certs = space_certs(cloud, &bob.token, space);
    accept_next(&before, &after, &certs, bob.account, &bob.master)
        .unwrap_or_else(|err| panic!("Bob's device refuses the revision: {err:?}"));
    assert!(
        own_record(cloud, &scene.secrets.token).is_none(),
        "the server takes the removed agent's token"
    );
    let mark = scene.mark();
    let refused = scene.agent.send(channel, AFTER_SAYS);
    assert!(!refused.success, "the removed agent sent: {refused:?}");
    scene.assert_none_sent(mark, "a run of the removed agent");
    let got = scene.agent.recv();
    assert!(!got.success, "the removed agent received: {got:?}");

    if !revoked(cloud, ada, scene.device) {
        revoked_by_cli(&ada.machine, scene.device);
    }
    assert!(
        revoked(cloud, ada, scene.device),
        "the removed agent's device is not revoked"
    );
}

/// The member agent `ci/deploy-bot` is in the channel's `services`. Ada's machine removes it: the
/// next revision lists its device neither in an entry nor in any channel's `services`, and its
/// token is refused.
#[test]
fn removing_a_member_agent_drops_its_device_from_services() {
    let bot = Bot::created(
        [3_135_911_001, 3_135_911_002],
        ["ada-removemember-standin", "bob-removemember-standin"],
    );
    let (cloud, space, ada) = (bot.talk.cloud(), bot.talk.space.id, &bot.talk.ada);
    let listed = latest(cloud, &ada.token, space);
    assert!(in_services(&listed, bot.device), "{listed:?}");

    let removed = remove_agent(ada);

    assert!(removed.success, "{removed:?}");
    let after = latest(cloud, &ada.token, space);
    assert_eq!(after.body().signer, ada.device, "{after:?}");
    assert!(
        !lists(&after, bot.device),
        "the agent is still listed: {after:?}"
    );
    assert!(
        own_record(cloud, &bot.secrets.token).is_none(),
        "the server takes the removed agent's token"
    );
}
