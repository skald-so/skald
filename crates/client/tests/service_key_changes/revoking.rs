//! Which device revokes a removed service agent's device: the removing device, for an agent of its
//! own account; for another account's agent, that account, with `skald revoke-device`.

use crate::changes::{lists, remove_agent, revoked};
use crate::fixtures::fronted;
use crate::kit::latest;
use crate::revocations::revoked_by_cli;
use crate::service_kit::{Acme, BOT, create_agent, secrets_of, service_of};
use crate::world::ACME;

/// Ada's machine creates `ci/deploy-bot`, then removes it: the server holds the revocation of its
/// device as soon as the command ends, with no `skald revoke-device`.
#[test]
fn removing_an_agent_of_the_device_s_account_revokes_its_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_940_001, "ada-ownremoval-standin", "ada-workstation");
    let acme = Acme::create(cloud, &ada);
    secrets_of(&create_agent(&ada.machine, ACME, BOT, &[acme.default], &[]));
    let (device, _) = service_of(cloud, &ada);

    let removed = remove_agent(&ada);

    assert!(removed.success, "{removed:?}");
    let after = latest(cloud, &ada.token, acme.id);
    assert!(
        !lists(&after, device),
        "the agent is still listed: {after:?}"
    );
    assert!(
        revoked(cloud, &ada, device),
        "the removal did not revoke the agent's device: {removed:?}"
    );
}

/// Bob's machine creates `acme`, with Ada a member, and Ada's account has the service agent
/// `ci/deploy-bot`, set up by hand. Bob's machine removes it: the next revision does not list its
/// device, which stays unrevoked, and the command says Ada's account revokes it. Ada's machine then
/// revokes it with `skald revoke-device`.
#[test]
fn another_account_s_admin_removes_the_agent_and_its_account_revokes_the_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let bob = world.sign_up(3_135_941_001, "bob-otherremoval-standin", "bob-laptop");
    let ada = world.sign_up(3_135_941_002, "ada-otherremoval-standin", "ada-workstation");
    let acme = Acme::with_member(cloud, &bob, &ada);
    let device = crate::changes::agent_of(cloud, (&ada, &bob), acme.id, acme.default);

    let removed = remove_agent(&bob);

    assert!(removed.success, "{removed:?}");
    let after = latest(cloud, &bob.token, acme.id);
    assert_eq!(after.body().signer, bob.device, "{after:?}");
    assert!(
        !lists(&after, device),
        "the agent is still listed: {after:?}"
    );
    assert!(
        !revoked(cloud, &ada, device),
        "Bob's machine revoked a device of Ada's account"
    );
    assert!(removed.stdout.contains("revoke-device"), "{removed:?}");

    revoked_by_cli(&ada.machine, device);
    assert!(
        revoked(cloud, &ada, device),
        "Ada's revoke-device did not revoke the agent's device"
    );
}
