//! An invite's acceptance on several instances: each instance sends its `space-notice` only to the
//! connections of the space's admins, also one whose listening connection dropped, once it listens
//! again, and none of an acceptance it knew of.

use crate::common::devices::assert_taken;
use crate::instances::{RELISTEN, Role, WAKE, instance, relisten_quietly, sender, watch};
use crate::scene::{Made, Scene, acceptance};
use crate::watch::Watch;

/// eve's acceptance, stored through the first instance, sends a notice of `acme` to bob's laptop on
/// a second instance, an admin's, and none to carol's there, a member's who is no admin.
#[test]
fn another_instance_sends_an_acceptances_notice_only_to_the_admins() {
    let scene = Scene::new(3_121_290_151);
    let t = scene.team();
    let other = instance(&scene.f, &[]);
    let invite = scene.invite();
    let mut bob = watch(&other, &scene.f, &t.bob);
    let mut carol = watch(&other, &scene.f, &t.carol);

    accept(&scene, &invite);

    assert_eq!(bob.notice_within(WAKE), scene.space());
    bob.assert_quiet();
    carol.assert_quiet();
    drop((bob, carol));
    drop(other);
}

/// While an instance does not listen, eve accepts an invite through another. Once it listens
/// again, it sends bob's laptop, an admin's, the notice of `acme`, and carol's, a member's who is no
/// admin, none; once it listens again after that, no notice of it.
#[test]
fn an_acceptance_an_instance_missed_reaches_only_the_admins_once_it_listens_again() {
    on_listening(3_121_290_161, |scene, invite, role, bob| {
        role.set_login(false);
        role.end_listening();
        accept(scene, invite);
        role.set_login(true);
        assert_eq!(bob.notice_within(RELISTEN), scene.space());
    });
}

/// Once an instance knows of an acceptance, told by another instance, it sends no notice of it when
/// it listens again.
#[test]
fn an_instance_that_listens_again_sends_no_notice_of_an_acceptance_it_knew_of() {
    on_listening(3_121_290_171, |scene, invite, _, bob| {
        accept(scene, invite);
        assert_eq!(bob.notice_within(WAKE), scene.space());
    });
}

/// eve's session accepts `invite` through the first instance.
#[track_caller]
fn accept(scene: &Scene, invite: &Made) {
    let eve = &scene.f.eve;
    assert_taken(&eve.session.accept(invite.id, &acceptance(invite, eve)));
}

/// On an instance with a database role of its own, `act` runs with an invite of `acme` while bob's
/// and carol's laptops watch; then bob's gets nothing more, also after the instance listens again,
/// and carol's nothing at all.
#[track_caller]
fn on_listening(github: u64, act: impl FnOnce(&Scene, &Made, &Role, &mut Watch)) {
    let scene = Scene::new(github);
    let t = scene.team();
    let invite = scene.invite();
    let role = Role::create(scene.f.db());
    let listening = role.instance(&scene.f);
    let mut bob = watch(&listening, &scene.f, &t.bob);
    let mut carol = watch(&listening, &scene.f, &t.carol);
    let mut alice = sender(&t.flow.stand.server, &scene.f, &t.alice);

    act(&scene, &invite, &role, &mut bob);

    bob.assert_quiet();
    relisten_quietly(&role, &scene.f, &mut alice, &mut bob, 1);
    carol.assert_quiet();
    drop((bob, carol, alice));
    drop(listening);
}
