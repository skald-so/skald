//! `skald set-domain` and `skald clear-domain`: an admin's device names the domain in the space's
//! next revision and clears it by the one after; on the device of a member that is no admin each
//! refuses and sends nothing. A new space's policy names no domain.

use skald_crypto::accept_next;
use skald_proto::{Role, Roster};

use crate::cloud::Cloud;
use crate::edits::{SPACE_CHANGE, assert_refused_as_no_admin, last_two};
use crate::kit::{latest, rosters};
use crate::world::{
    ACME, MEMBER, Person, assert_role, members, next_by, next_with, run, set_role, space_certs,
    store,
};
use crate::{
    CLEAR_DOMAIN, DOMAIN, LONG_TTL, SET_DOMAIN, Scene, assert_shows_domain, holds_domain, listed,
};

/// A domain an admin could name instead.
const OTHER_DOMAIN: &str = "other-m3x8.example";

/// Fails unless `next`, a revision the server holds, is the one after `previous` that `signer`'s
/// device signed with the policy's `domain` `domain` and no other change, and the signer accepts
/// it.
#[track_caller]
fn assert_domain(
    cloud: &Cloud,
    (previous, next): (&Roster, &Roster),
    signer: &Person,
    domain: Option<&str>,
) {
    let expected = next_with(previous, (signer.device, &signer.key), |body| {
        body.policy.domain = domain.map(str::to_owned);
        body.timestamp = next.body().timestamp;
    });
    assert_eq!(
        next.body(),
        expected.body(),
        "not the next revision, signed by the signer's device, with the domain alone"
    );
    let certs = space_certs(cloud, &signer.token, previous.body().space);
    if let Err(err) = accept_next(previous, next, &certs, signer.account, &signer.master) {
        panic!("the signer does not accept it: {err:?}");
    }
}

/// `acme`'s revisions, all of which Bob's machine made, name no domain. Bob's `skald set-domain
/// acme <domain>` has the server hold the next revision, signed by his device, with the domain and
/// no other change, and `skald members acme` shows the domain. His `skald clear-domain acme` then
/// has it hold the next one without the domain, and `skald members acme` no longer shows it.
#[test]
fn an_admin_sets_and_clears_the_domain() {
    let scene = Scene::new(
        [4_187_101_001, 4_187_101_002],
        ["bob-names-a-domain", "ada-in-a-named-space"],
    );
    let (cloud, bob) = (&scene.world.cloud, &scene.bob);
    let space = scene.third.body().space;
    scene.records(LONG_TTL, &[listed(bob), listed(&scene.ada)]);
    let made = rosters(cloud, &bob.token, space);
    for revision in &made[..2] {
        assert_eq!(
            revision.body().policy.domain,
            None,
            "a new space names a domain"
        );
    }

    let set = run(&bob.machine, &[SET_DOMAIN, ACME, DOMAIN]);

    assert!(set.success, "{set:?}");
    let (previous, next) = last_two(cloud, &bob.token, space, made.len());
    assert_domain(cloud, (&previous, &next), bob, Some(DOMAIN));
    assert_shows_domain(&scene.members(), &scene.handles());

    let cleared = run(&bob.machine, &[CLEAR_DOMAIN, ACME]);

    assert!(cleared.success, "{cleared:?}");
    let (previous, next) = last_two(cloud, &bob.token, space, made.len() + 1);
    assert_domain(cloud, (&previous, &next), bob, None);
    let shown = scene.members();
    assert!(
        !shown.stdout.lines().any(holds_domain),
        "{DOMAIN} is still shown: {shown:?}"
    );
}

/// Bob's `skald set-domain acme <domain>` names the domain while he is an admin. His device then
/// signs the revision that makes Ada an admin and Bob a member, which it accepts. There `skald
/// set-domain acme <another domain>` and `skald clear-domain acme` each fail, saying Bob is no
/// admin, and send nothing.
#[test]
fn the_device_of_a_member_that_is_no_admin_refuses_and_sends_nothing() {
    let scene = Scene::new(
        [4_187_102_001, 4_187_102_002],
        ["bob-no-admin-for-domains", "ada-admin-for-domains"],
    );
    let (cloud, bob, ada) = (&scene.world.cloud, &scene.bob, &scene.ada);
    let space = scene.third.body().space;
    scene.records(LONG_TTL, &[listed(bob), listed(ada)]);
    let set = run(&bob.machine, &[SET_DOMAIN, ACME, DOMAIN]);
    assert!(set.success, "{set:?}");
    let handed_over = next_by(&latest(cloud, &bob.token, space), bob, |list| {
        set_role(list, ada.account, Role::Admin);
        set_role(list, bob.account, Role::Member);
    });
    store(cloud, &bob.token, &handed_over);
    assert_role(&members(&bob.machine, ACME), &bob.handle, MEMBER);
    let held = rosters(cloud, &bob.token, space);
    let since = cloud.relay.mark();

    let refused = [
        run(&bob.machine, &[SET_DOMAIN, ACME, OTHER_DOMAIN]),
        run(&bob.machine, &[CLEAR_DOMAIN, ACME]),
    ];

    for one in &refused {
        assert_refused_as_no_admin(one);
    }
    let now = (
        cloud.count_sent(since, SPACE_CHANGE),
        rosters(cloud, &bob.token, space),
    );
    assert_eq!(now, (0, held), "it sent something: {refused:?}");
}
