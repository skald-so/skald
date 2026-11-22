//! Who gets an invite by its ID: any device session until the invite has an acceptance, then only
//! the sessions of that acceptance's account, which also get the space as a member's do. A service
//! token, no session and an unknown ID get nothing.

use skald_proto::{DeviceKind, InviteId};

use crate::api::{Session, assert_refused};
use crate::common::device_flow::text;
use crate::common::devices::{assert_taken, b64};
use crate::scene::{Scene, acceptance, gets_no_invite, made};
use crate::tokens::Stage;

/// With a revocation of bob's in `acme`, the device sessions of eve and frank, outside the space,
/// of carol, a member, and of alice, its admin, each name the invite's ID and get it byte for byte
/// with `acme`'s revisions, devices and revocations.
#[test]
fn a_device_session_that_names_the_id_gets_the_invite_with_the_spaces_revisions_and_devices() {
    let scene = Scene::new(3_817_200_001);
    let t = scene.team();
    let bob = &t.bob;
    let tablet = bob.certified_by(scene.f.db(), &bob.laptop, DeviceKind::Client, "Tablet");
    bob.revoke(&bob.by_device(&bob.laptop, tablet.id));

    let invite = scene.invite();
    for person in [&scene.f.eve, &scene.frank, &t.carol, &t.alice] {
        scene.gets_invite(person, &invite);
    }
}

/// A service token of alice's `service` device and a request without a session are refused the
/// invite with `401`; eve's session is refused an ID never issued and one issued but holding no
/// invite. None of the answers holds the invite.
#[test]
fn a_service_token_no_session_and_an_unknown_id_get_nothing() {
    let stage = Stage::new(3_817_200_101);
    let f = &stage.f;
    let alice = &f.team.alice;
    let space = stage.space();
    let invite = made(space, alice, &alice.laptop, vec![(f.general, true)], |_| {});
    assert_taken(&alice.session.put_invite(space, &invite));
    let created = stage.create();

    for (session, what) in [
        (
            Session::of_token(&f.team.flow, &created.token),
            "a service token",
        ),
        (Session::none(&f.team.flow), "no session"),
    ] {
        let response = session.invite_response(invite.id);
        assert_eq!(response.status, 401, "{what}: {}", text(&response));
        assert_refused(&response, what);
        assert!(!text(&response).contains(&b64(&invite.bytes)), "{what}");
    }
    gets_no_invite(&f.eve, InviteId::generate(), None, "an ID never issued");
    let empty = alice.session.issue(space);
    gets_no_invite(&f.eve, empty, None, "an ID with no invite");
}

/// Once eve accepts the invite, only eve's session gets it, and it gets `acme`'s revisions and
/// devices as a member's does, which it was refused before; frank, outside the space, and carol and
/// dave, members who are no admins, get no invite, and frank no revisions of `acme`.
#[test]
fn after_an_acceptance_only_its_accounts_sessions_get_the_invite_and_the_space() {
    let scene = Scene::new(3_817_200_201);
    let t = scene.team();
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let space = scene.space();
    let invite = scene.invite();
    assert_refused(&eve.session.revisions_response(space), "eve, before");
    assert_refused(&eve.session.devices_response(space), "eve, before");

    assert_taken(&eve.session.accept(invite.id, &acceptance(&invite, eve)));

    scene.gets_invite(eve, &invite);
    let alice = &t.alice.session;
    assert_eq!(eve.session.revisions(space), alice.revisions(space), "eve");
    assert_eq!(eve.session.devices(space), alice.devices(space), "eve");
    for person in [frank, &t.carol, &t.dave] {
        gets_no_invite(person, invite.id, Some(&invite), "after an acceptance");
    }
    assert_refused(&frank.session.revisions_response(space), "frank");
}
