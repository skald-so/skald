//! What blocks a deletion: a space where the account is the only admin and there are other
//! members. Such a deletion is refused with nothing deleted, a deletion and a revision that would
//! together leave a space without a living admin never both succeed, and once the account is
//! deleted, a revision that leaves its spaces without a living admin is refused.

use std::sync::Barrier;
use std::thread;

use skald_proto::Role;

use crate::api::Team;
use crate::common::devices::assert_taken;
use crate::deletion::{
    Preview, acme_with_member, assert_blocked, assert_deleted, assert_holds_space, delete_by_token,
    deletes, ids, personal_space, preview, with_role,
};
use crate::rosters::{member, next};
use crate::sessions::Api;

/// alice created `acme` and is its only admin, bob a member; she also has a personal space. The
/// server says her deletion deletes the personal space and `acme` blocks it; her deletion is
/// refused with `409` and the problem `deletion-blocked` naming `acme`, and deletes nothing. Once a
/// revision makes bob an admin too, she deletes her account.
#[test]
fn only_admin_of_a_space_with_other_members_cannot_delete_the_account() {
    let team = Team::new(3_303_060_001);
    let (alice, bob) = (&team.alice, &team.bob);
    let [one, two] = acme_with_member(&team, bob);
    let personal = personal_space(alice);
    let api = Api::of(&team.flow);
    let expected = Preview {
        deletes: ids(&[personal]),
        blocks: ids(&[team.space]),
    };
    assert_eq!(preview(&api, alice), expected);

    assert_blocked(&delete_by_token(&api, alice.session.token()), &[team.space]);

    assert!(api.device_works(alice.session.token()), "alice's session");
    assert_eq!(preview(&api, alice), expected, "after the refusal");
    assert_holds_space(&team.flow.stand.db, personal, "the personal space");
    assert_eq!(bob.session.revisions(team.space), [one, two.clone()]);
    assert!(bob.session.spaces().contains_key(&team.space.to_string()));

    let mut body = next(&two, &alice.laptop);
    with_role(&mut body, bob, Role::Admin);
    team.stored_by(alice, &alice.sign(&body));
    deletes(&api, alice);
}

/// In `acme`, alice and bob are admins and carol a member. alice's deletion and bob's revision
/// making himself a member, sent at once, never both succeed: either the deletion is blocked by
/// `acme` and the revision stored, or the account is deleted and the revision refused.
#[test]
fn deletion_and_a_revision_leaving_no_other_admin_never_both_succeed() {
    let team = Team::new(3_303_060_011);
    let (alice, bob) = (&team.alice, &team.bob);
    let [_, two] = team.with_members();
    let mut body = next(&two, &bob.laptop);
    with_role(&mut body, bob, Role::Member);
    let demoted = bob.sign(&body);
    let api = Api::of(&team.flow);
    let (base, public, db) = (api.base, api.public, api.db);
    let start = Barrier::new(2);

    let (deletion, revision) = thread::scope(|scope| {
        let deleting = scope.spawn(|| {
            let api = Api { base, public, db };
            start.wait();
            delete_by_token(&api, alice.session.token())
        });
        let revising = scope.spawn(|| {
            start.wait();
            bob.session.store(team.space, &demoted)
        });
        (
            deleting.join().expect("the deletion"),
            revising.join().expect("the revision"),
        )
    });

    let stored = matches!(revision.status, 200 | 201);
    if deletion.status == 204 {
        assert_deleted(&deletion);
        assert!(!stored, "both succeeded: {}", revision.status);
        crate::deletion::assert_refused(&revision, "the revision after the deletion");
        assert_eq!(bob.session.revisions(team.space).last(), Some(&two));
    } else {
        assert_blocked(&deletion, &[team.space]);
        assert_taken(&revision);
        assert!(api.device_works(alice.session.token()), "alice kept");
        assert_eq!(bob.session.revisions(team.space).last(), Some(&demoted));
    }
}

/// In `acme`, alice and bob are admins and carol a member. Once alice deletes her account, bob's
/// revision making himself a member is refused, while his revision adding dave as a member is
/// stored.
#[test]
fn after_the_deletion_a_revision_leaving_no_living_admin_is_refused() {
    let team = Team::new(3_303_060_021);
    let (alice, bob) = (&team.alice, &team.bob);
    let [one, two] = team.with_members();
    let api = Api::of(&team.flow);

    deletes(&api, alice);

    let mut body = next(&two, &bob.laptop);
    with_role(&mut body, bob, Role::Member);
    let stored = [one, two.clone()];
    let refused = bob.session.store(team.space, &bob.sign(&body));
    crate::deletion::assert_refused(&refused, "bob made a member");
    assert_eq!(bob.session.revisions(team.space), stored);

    let mut body = next(&two, &bob.laptop);
    body.members.push(member(&team.dave, Role::Member));
    team.stored_by(bob, &bob.sign(&body));
}
