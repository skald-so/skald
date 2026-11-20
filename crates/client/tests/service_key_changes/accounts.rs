//! What a revision an admin's device signs keeps of service devices: removing a member's account
//! drops its agents in the same revision, and the next revision after a `service` device's
//! revocation drops it.

use skald_proto::Roster;

use crate::changes::{agent_of, lists};
use crate::edits::REMOVE_MEMBER;
use crate::fixtures::{fronted, last_channel};
use crate::member::MEMBER;
use crate::revocations::revoked_by_cli;
use crate::service_kit::{Acme, BOT, create_agent, rosters, secrets_of, service_of};
use crate::world::{ACME, CREATE_CHANNEL, run};

/// Fails unless `roster` lists no member of `account`.
#[track_caller]
fn assert_no_member(roster: &Roster, account: skald_proto::AccountId) {
    let body = roster.body();
    assert!(
        body.members.iter().all(|member| member.account != account),
        "{body:?}"
    );
}

/// Bob, a member of `acme`, has the service agent `ci/deploy-bot` on his account, in the default
/// channel's `services` too, in a revision Ada's device signed. Ada's machine removes Bob from
/// `acme`: that one revision drops Bob and lists his agent's device neither in an entry nor in
/// `services`, and the server takes it.
#[test]
fn removing_the_agent_s_account_from_the_space_drops_its_agents_in_the_same_revision() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_920_001, "ada-dropaccount-standin", "ada-workstation");
    let bob = world.sign_up(3_135_920_002, "bob-dropaccount-standin", "bob-laptop");
    let acme = Acme::with_member(cloud, &ada, &bob);
    let device = agent_of(cloud, (&bob, &ada), acme.id, acme.default);
    let before = rosters(cloud, &ada.token, acme.id);

    let removed = run(&ada.machine, &[REMOVE_MEMBER, ACME, bob.handle.as_str()]);

    assert!(removed.success, "{removed:?}");
    let after = rosters(cloud, &ada.token, acme.id);
    assert_eq!(
        after.len(),
        before.len() + 1,
        "not one new revision: {after:?}"
    );
    let last = after.last().expect("a revision");
    assert_no_member(last, bob.account);
    assert!(
        !lists(last, device),
        "Bob's agent is still listed: {last:?}"
    );
}

/// Ada's machine creates the member agent `ci/deploy-bot`, then revokes its device. The next
/// revision her machine signs, which adds a channel, lists the device neither in an entry nor in
/// `services`, and the server takes it.
#[test]
fn the_next_revision_an_admin_signs_drops_a_revoked_service_device() {
    let (world, _front) = fronted();
    let cloud = &world.cloud;
    let ada = world.sign_up(3_135_921_001, "ada-droprevoked-standin", "ada-workstation");
    let acme = Acme::create(cloud, &ada);
    secrets_of(&create_agent(
        &ada.machine,
        ACME,
        BOT,
        &[acme.default],
        &[MEMBER],
    ));
    let (device, _) = service_of(cloud, &ada);
    revoked_by_cli(&ada.machine, device);
    let before = rosters(cloud, &ada.token, acme.id);

    let created = run(&ada.machine, &[CREATE_CHANNEL, ACME]);

    assert!(created.success, "{created:?}");
    let (last, channel) = last_channel(cloud, &ada, acme.id);
    assert_eq!(last.body().revision, before.len() as u64 + 1, "{last:?}");
    assert!(
        before
            .iter()
            .all(|roster| roster.body().channels.iter().all(|it| it.id != channel)),
        "not a new channel: {channel}"
    );
    assert!(
        !lists(&last, device),
        "the revoked device is still listed: {last:?}"
    );
}
