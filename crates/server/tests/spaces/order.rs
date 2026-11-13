//! The order of a space's revisions: revision 1, then n+1 only after n, each stored as the bytes the
//! server got; the same bytes again count as stored; of two revisions with the same number the
//! first wins.

use std::sync::Barrier;
use std::thread;

use skald_proto::{Cbor, Role, Roster, SpaceId};

use crate::api::{ACME, Team};
use crate::common::Response;
use crate::common::device_flow::text;
use crate::rosters::{channel, first, member, next, signed_bytes, with_later_field};

#[test]
fn revisions_one_to_three_are_stored_in_order_and_given_back_byte_for_byte() {
    let team = Team::new(3_120_810_001);
    let (alice, bob, carol) = (&team.alice, &team.bob, &team.carol);

    // Revision 1 with a field of a later version, which the server keeps as it got it.
    let body = first(team.space, ACME, alice).to_cbor();
    let first = signed_bytes(&with_later_field(&body), &alice.laptop.signing.key);
    team.stored_by(alice, &first);
    assert_eq!(
        alice.session.revisions(team.space),
        std::slice::from_ref(&first)
    );

    let mut body = next(&first, &alice.laptop);
    body.members.push(member(bob, Role::Admin));
    body.members.push(member(carol, Role::Member));
    let second = alice.sign(&body);
    team.stored_by(alice, &second);

    // Revision 3 from another admin's session, signed by its device, with a channel.
    let mut body = next(&second, &bob.laptop);
    let id = bob.session.channel(team.space);
    body.channels.push(channel(id, &[alice, bob, carol]));
    let third = bob.sign(&body);
    team.stored_by(bob, &third);

    let stored = vec![first, second, third];
    for person in [alice, bob, carol] {
        assert_eq!(person.session.revisions(team.space), stored);
    }
}

#[test]
fn the_same_revision_sent_again_is_taken_and_stored_once() {
    let team = Team::new(3_120_810_011);
    let alice = &team.alice;
    let mut stored = Vec::new();
    let mut revision = alice.sign(&first(team.space, ACME, alice));
    for _ in 1..=3 {
        // A retry after a lost answer.
        team.stored_by(alice, &revision);
        team.stored_by(alice, &revision);
        stored.push(revision.clone());
        assert_eq!(alice.session.revisions(team.space), stored);
        revision = alice.sign(&next(&revision, &alice.laptop));
    }
}

/// Sends `revisions` to `space` from their sessions all at once; gives the answers in order.
fn sent_together(space: SpaceId, revisions: &[(&crate::api::Person, Vec<u8>)]) -> Vec<Response> {
    let start = Barrier::new(revisions.len());
    thread::scope(|scope| {
        let threads: Vec<_> = revisions
            .iter()
            .map(|(person, revision)| {
                let (start, session) = (&start, person.session.clone());
                scope.spawn(move || {
                    start.wait();
                    session.store(space, revision)
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().expect("a request"))
            .collect()
    })
}

#[test]
fn of_two_revisions_with_the_same_number_sent_at_once_one_is_stored_and_the_other_refused() {
    let team = Team::new(3_120_810_021);
    let (alice, bob) = (&team.alice, &team.bob);
    let mut stored = team.with_members().to_vec();

    for round in 0..5 {
        let latest = stored.last().expect("a revision").clone();
        let mut by_alice = next(&latest, &alice.laptop);
        by_alice.timestamp += round;
        let mut by_bob = next(&latest, &bob.laptop);
        by_bob.timestamp += round;
        let candidates = [(alice, alice.sign(&by_alice)), (bob, bob.sign(&by_bob))];

        let answers = sent_together(team.space, &candidates);
        let taken: Vec<_> = answers
            .iter()
            .map(|answer| matches!(answer.status, 200 | 201))
            .collect();
        assert_eq!(
            taken.iter().filter(|taken| **taken).count(),
            1,
            "round {round}: one taken: {:?}",
            answers.iter().map(text).collect::<Vec<_>>()
        );
        let winner = taken.iter().position(|taken| *taken).expect("one taken");
        crate::api::assert_refused(&answers[1 - winner], &format!("round {round}: the other"));
        stored.push(candidates[winner].1.clone());
        assert_eq!(alice.session.revisions(team.space), stored, "round {round}");
    }
}

#[test]
fn a_revision_out_of_order_is_refused_and_not_stored() {
    let team = Team::new(3_120_810_031);
    let (alice, bob) = (&team.alice, &team.bob);
    let first = alice.sign(&first(team.space, ACME, alice));

    // Revision 2 before revision 1.
    let mut body = next(&first, &alice.laptop);
    body.members.push(member(bob, Role::Admin));
    let second = alice.sign(&body);
    team.refused(alice, &second, &[], "revision 2 before 1");

    team.stored_by(alice, &first);
    let stored = vec![first.clone()];

    // Another revision 1 once one is stored.
    let mut other = crate::rosters::first(team.space, ACME, alice);
    other.timestamp += 1;
    team.refused(alice, &alice.sign(&other), &stored, "another revision 1");

    // Revision 3 before revision 2.
    let mut body = next(&second, &alice.laptop);
    body.timestamp += 1;
    team.refused(alice, &alice.sign(&body), &stored, "revision 3 before 2");

    // Revision 2 whose `prev` is not the hash of revision 1.
    let mut body = next(&first, &alice.laptop);
    body.prev = Some([0x42; 32]);
    team.refused(alice, &alice.sign(&body), &stored, "a wrong prev");
    let mut body = next(&first, &alice.laptop);
    body.prev = None;
    team.refused(alice, &alice.sign(&body), &stored, "no prev");

    team.stored_by(alice, &second);
    let stored = vec![first, second.clone()];

    // Another revision 2 once one is stored.
    let mut body = next(&stored[0], &alice.laptop);
    body.timestamp += 1;
    team.refused(alice, &alice.sign(&body), &stored, "another revision 2");

    // Revision 4 after revision 2, and revision 3 numbered 2 again.
    let mut body = next(&second, &alice.laptop);
    body.revision = 4;
    team.refused(alice, &alice.sign(&body), &stored, "revision 4 after 2");
    let mut body = next(&second, &alice.laptop);
    body.revision = 2;
    team.refused(alice, &alice.sign(&body), &stored, "revision 2 after 2");
}

#[test]
fn a_revision_of_another_space_or_name_is_refused_and_not_stored() {
    let team = Team::new(3_120_810_041);
    let (alice, bob) = (&team.alice, &team.bob);
    let other = bob.create_space("other");

    // Revision 1 of bob's space, or with another name, sent to alice's space.
    let of_other = first(other, "other", alice);
    team.refused(alice, &alice.sign(&of_other), &[], "another space");
    let renamed = first(team.space, "other", alice);
    team.refused(alice, &alice.sign(&renamed), &[], "another name");
    assert!(
        bob.session.revisions(other).is_empty(),
        "bob's space has no revision"
    );

    let revisions = team.with_members();
    let mut body = next(&revisions[1], &alice.laptop);
    body.space = other;
    team.refused(
        alice,
        &alice.sign(&body),
        &revisions,
        "revision 3 of another space",
    );
    let mut body = next(&revisions[1], &alice.laptop);
    body.name = crate::rosters::name("other");
    team.refused(
        alice,
        &alice.sign(&body),
        &revisions,
        "revision 3 with another name",
    );
}

#[test]
fn bytes_that_do_not_decode_are_refused_and_not_stored() {
    let team = Team::new(3_120_810_051);
    let alice = &team.alice;
    let first = alice.sign(&first(team.space, ACME, alice));

    let roster = Roster::from_cbor(&first).expect("a revision that decodes");
    let truncated = &first[..first.len() - 1];
    let mut trailing = first.clone();
    trailing.push(0x00);
    let cases: [(&str, &[u8]); 5] = [
        ("no bytes", &[]),
        ("not CBOR", b"not a revision"),
        ("a truncated revision", truncated),
        ("a revision with a byte after it", &trailing),
        ("its body alone", roster.body_bytes()),
    ];
    for (what, bytes) in cases {
        team.refused(alice, bytes, &[], what);
    }

    team.stored_by(alice, &first);
    let stored = vec![first.clone()];
    let second = alice.sign(&next(&first, &alice.laptop));
    let truncated = &second[..second.len() / 2];
    team.refused(alice, truncated, &stored, "a truncated revision 2");
}
