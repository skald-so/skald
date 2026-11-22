//! Invites Bob's device refuses after taking them by their ID, saying why, without asking to
//! confirm, signing nothing and following nothing: a revision a proxy swaps for the link's,
//! revisions off its chain, an invite signed by a device of a member who is no admin or by one only
//! the server certified, a link whose secret does not derive `key`, an expired invite, an account
//! already a member, and an invite the server no longer holds.

use skald_crypto::InviteSecret;
use skald_proto::{DeviceKind, Role, Roster};

use crate::fixtures::{Issuer, add_device, add_device_of_kind};
use crate::world::{next_by, next_with, now, store};
use crate::{
    Made, Scene, assert_refused_saying, give_invite, give_revisions, link_of, refused_at_terminal,
    resigned,
};

/// The revisions of `acme` the server holds, the two the scene made; fails if it holds others.
#[track_caller]
fn two_revisions(scene: &Scene) -> Vec<Roster> {
    let held = crate::kit::rosters(scene.cloud(), &scene.ada.token, scene.space.id);
    assert_eq!(held.len(), 2, "{held:?}");
    held
}

/// The relay gives `revisions` with `made`; `skald accept-invite`, given its link, fails saying
/// `revision`, and Bob's machine accepts nothing.
#[track_caller]
fn assert_revisions_refused(scene: &Scene, made: &Made, revisions: &[&Roster]) {
    give_revisions(scene.cloud(), made.id(), revisions);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["revision"]);
    scene.assert_nothing_accepted(made, since);
}

/// The relay gives, among `acme`'s revisions with the invite, a revision 2 that Ada's device
/// signed after revision 1 in place of the revision 2 the link names. `skald accept-invite`, given
/// the link, fails saying `revision`, and Bob's machine accepts nothing.
#[test]
fn revision_a_proxy_swaps_for_the_link_s_is_refused() {
    let scene = Scene::new(
        (7_305_402_001, "ada-whose-revision-is-swapped"),
        (7_305_402_002, "bob-sees-a-swapped-revision"),
    );
    let made = scene.invite();
    let held = two_revisions(&scene);
    let swapped = next_with(&held[0], (scene.ada.device, &scene.ada.key), |_| {});
    assert_ne!(swapped.hash(), made.revision.hash());

    assert_revisions_refused(&scene, &made, &[&held[0], &swapped]);
}

/// The relay gives, after `acme`'s revisions 1 and 2, the one the link names, a revision 3 signed
/// by Ada's device whose `prev` is the hash of revision 1: off the link revision's chain.
/// `skald accept-invite`, given the link, fails saying `revision`, and Bob's machine accepts
/// nothing.
#[test]
fn revisions_off_the_link_revision_s_chain_are_refused() {
    let scene = Scene::new(
        (7_305_402_101, "ada-whose-roster-forks"),
        (7_305_402_102, "bob-sees-a-fork"),
    );
    let made = scene.invite();
    let held = two_revisions(&scene);
    let first = held[0].hash();
    let off = next_with(&held[1], (scene.ada.device, &scene.ada.key), |body| {
        body.prev = Some(first);
    });

    assert_revisions_refused(&scene, &made, &[&held[0], &held[1], &off]);
}

/// Revision 3 of `acme`, signed by Ada's device, adds Carol as a member; the invite's link names
/// it. The relay gives the invite signed by Carol's device, naming it as `signer`. `skald
/// accept-invite`, given the link, fails saying `admin`, and Bob's machine accepts nothing.
#[test]
fn invite_signed_by_a_device_of_a_member_who_is_no_admin_is_refused() {
    let scene = Scene::new(
        (7_305_402_201, "ada-with-a-member"),
        (7_305_402_202, "bob-invited-by-a-member"),
    );
    let carol = scene
        .world
        .sign_up(7_305_402_203, "carol-no-admin", "carol-laptop");
    let third = next_by(&scene.latest(), &scene.ada, |members| {
        members.push(carol.member(Role::Member));
    });
    store(scene.cloud(), &scene.ada.token, &third);
    let made = scene.invite();
    assert_eq!(made.revision.hash(), third.hash());
    let by_carol = resigned(&made, &carol.key, |body| body.signer = carol.device);
    give_invite(scene.cloud(), &by_carol);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["admin"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The server's database holds a `client` device of Ada's account whose certificate a key of no
/// device signed, as only the server could. The relay gives the invite signed by that device,
/// naming it as `signer`. `skald accept-invite`, given the link, fails saying `admin` or
/// `certificate`, and Bob's machine accepts nothing.
#[test]
fn invite_signed_by_a_device_only_the_server_certified_is_refused() {
    let scene = Scene::new(
        (7_305_402_301, "ada-with-a-server-made-device"),
        (7_305_402_302, "bob-invited-by-the-server"),
    );
    let made = scene.invite();
    let made_up = add_device(
        scene.cloud(),
        scene.ada.account,
        "ada-server-made",
        &Issuer::Server,
    );
    let by_server = resigned(&made, &made_up.signing, |body| body.signer = made_up.id);
    give_invite(scene.cloud(), &by_server);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["admin", "certificate", "certified"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The link of an invite the server holds, with another secret than the one that derives its
/// `key`. `skald accept-invite`, given it, fails saying `secret` or `key`, and Bob's machine accepts
/// nothing.
#[test]
fn link_whose_secret_does_not_derive_the_invite_s_key_is_refused() {
    let scene = Scene::new(
        (7_305_402_401, "ada-sends-a-wrong-secret"),
        (7_305_402_402, "bob-gets-a-wrong-secret"),
    );
    let made = scene.invite();
    let other = *InviteSecret::generate().to_bytes();
    let link = link_of(
        &scene.cloud().public,
        made.id(),
        &other,
        made.revision.hash(),
    );
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &link);

    assert_refused_saying(&refused, &["secret", "key"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The relay gives the invite signed by Ada's device with an `expires` a minute ago. `skald
/// accept-invite`, given the link, fails saying `expired`, and Bob's machine accepts nothing.
#[test]
fn expired_invite_is_refused() {
    let scene = Scene::new(
        (7_305_402_501, "ada-whose-invite-expired"),
        (7_305_402_502, "bob-comes-too-late"),
    );
    let made = scene.invite();
    let expired = resigned(&made, &scene.ada.key, |body| body.expires = now() - 60);
    give_invite(scene.cloud(), &expired);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["expired"]);
    scene.assert_nothing_accepted(&made, since);
}

/// Revision 3 of `acme`, signed by Ada's device, adds Bob as a member; then the invite. `skald
/// accept-invite`, given its link, fails saying `already`, and Bob's machine sends no acceptance.
#[test]
fn account_already_a_member_is_refused() {
    let scene = Scene::new(
        (7_305_402_601, "ada-invites-a-member-again"),
        (7_305_402_602, "bob-already-in-acme"),
    );
    let third = next_by(&scene.latest(), &scene.ada, |members| {
        members.push(scene.bob.member(Role::Member));
    });
    store(scene.cloud(), &scene.ada.token, &third);
    let made = scene.invite();
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["already"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The invite is deleted from the server's database before Bob accepts it. `skald accept-invite`,
/// given its link, fails saying `ended`, and Bob's machine accepts nothing.
#[test]
fn invite_the_server_no_longer_holds_is_refused_as_ended() {
    let scene = Scene::new(
        (7_305_402_701, "ada-whose-invite-ended"),
        (7_305_402_702, "bob-finds-it-ended"),
    );
    let made = scene.invite();
    scene
        .cloud()
        .execute(&format!("DELETE FROM invites WHERE id = '{}'", made.id()));
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["ended"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The link of an invite the server holds names a hash that none of `acme`'s revisions has.
/// `skald accept-invite`, given it, fails saying `revision`, and Bob's machine accepts nothing.
#[test]
fn link_naming_a_revision_the_space_lacks_is_refused() {
    let scene = Scene::new(
        (7_305_410_001, "ada-whose-link-names-no-revision"),
        (7_305_410_002, "bob-gets-a-link-to-no-revision"),
    );
    let made = scene.invite();
    let link = link_of(&scene.cloud().public, made.id(), &made.secret, [7; 32]);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &link);

    assert_refused_saying(&refused, &["revision"]);
    scene.assert_nothing_accepted(&made, since);
}

/// The relay gives the invite signed by a `service` device of Ada's, an admin, that her device
/// certified, naming it as `signer`: no admin's device, which is a `client` or `browser` device.
/// `skald accept-invite`, given the link, fails saying `browser`; Bob's machine accepts nothing.
#[test]
fn invite_signed_by_a_service_device_of_an_admin_is_refused() {
    let scene = Scene::new(
        (7_305_410_101, "ada-whose-ci-signs-an-invite"),
        (7_305_410_102, "bob-invited-by-ci"),
    );
    let made = scene.invite();
    let service = add_device_of_kind(
        scene.cloud(),
        (scene.ada.account, DeviceKind::Service),
        "ada-ci-runner",
        &Issuer::Device(&scene.ada),
    );
    let by_service = resigned(&made, &service.signing, |body| body.signer = service.id);
    give_invite(scene.cloud(), &by_service);
    let since = scene.cloud().relay.mark();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["browser"]);
    scene.assert_nothing_accepted(&made, since);
}

/// Bob accepts an invite to `acme`, which his device then follows. `skald accept-invite`, given
/// its link again, fails saying `already`; the server still holds his one acceptance.
#[test]
fn space_the_device_follows_already_is_refused() {
    let scene = Scene::new(
        (7_305_410_201, "ada-whose-link-bob-reuses"),
        (7_305_410_202, "bob-accepts-twice"),
    );
    let made = scene.bob_accepts();

    let refused = refused_at_terminal(&scene.bob.machine, &made.link);

    assert_refused_saying(&refused, &["already"]);
    scene.assert_bob_accepted(&made, scene.bob.handle.as_str());
}
