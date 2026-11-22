//! An invite ends when a stored revision lists a member with it or when its `expires` passes: the
//! server then gives and takes neither it nor its acceptance.

use skald_proto::Role;

use crate::api::assert_refused;
use crate::common::devices::assert_taken;
use crate::link::WAIT;
use crate::rosters::member;
use crate::scene::{Scene, acceptance, gets_no_invite, wait_past};
use crate::watch::Watch;
use crate::wire::now;

/// eve accepts an invite; once alice stores a revision that lists eve with it, eve's laptop gets
/// that revision's notice, no one gets the invite or eve's acceptance, and frank's acceptance is
/// refused. Another invite of `acme` stays open.
#[test]
fn once_a_stored_revision_lists_the_member_with_the_invite_it_and_its_acceptance_are_gone() {
    let mut scene = Scene::new(3_817_500_001);
    let invite = scene.invite();
    let open = scene.invite();
    let accepted = acceptance(&invite, &scene.f.eve);
    assert_taken(&scene.f.eve.session.accept(invite.id, &accepted));
    assert_eq!(scene.waiting_bytes(), vec![accepted]);
    let flow = &scene.f.team.flow;
    let mut eves = Watch::from(&flow.stand.server, flow, &scene.f.eve, 0, &[]);

    let mut admitted = member(&scene.f.eve, Role::Member);
    admitted.invite = Some(invite.id);
    scene.f.revise(|body| body.members.push(admitted));

    assert_eq!(eves.notice_within(WAIT), scene.space(), "eve's laptop");
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    for person in [eve, frank] {
        gets_no_invite(
            person,
            invite.id,
            Some(&invite),
            "once a revision lists eve",
        );
    }
    assert_eq!(scene.waiting_bytes(), Vec::<Vec<u8>>::new(), "waiting");
    let franks = acceptance(&invite, frank);
    assert_refused(&frank.session.accept(invite.id, &franks), "frank's");
    scene.gets_invite(frank, &open);
}

/// eve accepts an invite that expires in a few seconds; once `expires` has passed, alice's session
/// gets no waiting acceptance, eve's gets neither the invite nor `acme`'s revisions and devices,
/// and frank's acceptance is refused.
#[test]
fn past_expires_the_invite_and_its_acceptance_are_gone() {
    let scene = Scene::new(3_817_500_101);
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let space = scene.space();
    let expires = now() + 6;
    let invite = scene.invite_until(expires);
    let accepted = acceptance(&invite, eve);
    assert_taken(&eve.session.accept(invite.id, &accepted));
    assert_eq!(scene.waiting_bytes(), vec![accepted]);

    wait_past(expires);

    assert_eq!(scene.waiting_bytes(), Vec::<Vec<u8>>::new(), "waiting");
    gets_no_invite(eve, invite.id, Some(&invite), "past expires");
    assert_refused(&eve.session.revisions_response(space), "eve's revisions");
    assert_refused(&eve.session.devices_response(space), "eve's devices");
    let franks = acceptance(&invite, frank);
    assert_refused(&frank.session.accept(invite.id, &franks), "frank's");
}
