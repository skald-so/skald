//! Storing an invite's acceptance: the first one, from a session of the account it names, with the
//! master key the server holds for that account, signed by the invite key, before `expires`; once,
//! also when sent twice; of two at once, across instances too, one.

use std::sync::Barrier;
use std::thread;

use skald_crypto::InviteSecret;
use skald_proto::InviteId;

use crate::api::{Person, Session, assert_refused};
use crate::common::Response;
use crate::common::device_flow::text;
use crate::common::devices::Key;
use crate::scene::{
    MALFORMED, Scene, acceptance, acceptance_body, assert_stored, broken, signed_acceptance,
    wait_past,
};
use crate::wire::now;

/// eve's acceptance is stored, `201`, and taken as stored when sent again, `200`; alice's session
/// gets it as waiting once.
#[test]
fn an_acceptance_is_stored_once_also_when_sent_twice() {
    let scene = Scene::new(3_817_300_001);
    let eve = &scene.f.eve;
    let invite = scene.invite();
    let accepted = acceptance(&invite, eve);

    assert_stored(&eve.session.accept(invite.id, &accepted), true, "first");
    assert_stored(&eve.session.accept(invite.id, &accepted), false, "again");
    assert_eq!(scene.waiting_bytes(), vec![accepted]);
}

/// Refused, with nothing waiting: an acceptance with a broken signature, one signed by another
/// key, from eve's session one naming frank's account or another master key, eve's own from
/// frank's session, bytes that are no acceptance, and eve's sent for an ID with no invite. The
/// invite stays open: eve's acceptance is then stored.
#[test]
fn acceptances_that_fail_their_checks_are_refused_and_leave_the_invite_open() {
    let scene = Scene::new(3_817_300_101);
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let invite = scene.invite();
    let key = invite.secret.invite_key();
    let accepted = acceptance(&invite, eve);

    let mut naming_frank = acceptance_body(&invite, eve);
    naming_frank.account = frank.account;
    naming_frank.master = frank.master.public();
    let mut other_master = acceptance_body(&invite, eve);
    other_master.master = Key::generate().public();
    let other_key = InviteSecret::generate().invite_key();
    let by_other_key = signed_acceptance(&acceptance_body(&invite, eve), &other_key);

    let cases: [(&Person, Vec<u8>, &str); 6] = [
        (eve, broken(&accepted), "a broken signature"),
        (eve, by_other_key, "signed by another key"),
        (
            eve,
            signed_acceptance(&naming_frank, &key),
            "naming another account",
        ),
        (
            eve,
            signed_acceptance(&other_master, &key),
            "another master key",
        ),
        (frank, accepted.clone(), "from another account's session"),
        (eve, MALFORMED.to_vec(), "malformed"),
    ];
    for (person, bytes, what) in cases {
        assert_refused(&person.session.accept(invite.id, &bytes), what);
        assert_eq!(scene.waiting_bytes(), Vec::<Vec<u8>>::new(), "{what}");
    }
    let unknown = InviteId::generate();
    assert_refused(&eve.session.accept(unknown, &accepted), "an unknown invite");

    assert_stored(&eve.session.accept(invite.id, &accepted), true, "eve's");
    assert_eq!(scene.waiting_bytes(), vec![accepted]);
}

/// After eve's acceptance, frank's own acceptance of the invite is refused and eve's stays the one
/// waiting.
#[test]
fn another_accounts_acceptance_after_the_first_is_refused() {
    let scene = Scene::new(3_817_300_201);
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let invite = scene.invite();
    let eves = acceptance(&invite, eve);
    assert_stored(&eve.session.accept(invite.id, &eves), true, "eve's");

    let franks = acceptance(&invite, frank);
    assert_refused(&frank.session.accept(invite.id, &franks), "frank's");
    assert_eq!(scene.waiting_bytes(), vec![eves]);
}

/// Sends each of `sends`, an acceptance from a session, of the invite `id`, all at once; gives the
/// answers in order.
fn sent_together(id: InviteId, sends: &[(Session, Vec<u8>)]) -> Vec<Response> {
    let start = Barrier::new(sends.len());
    thread::scope(|scope| {
        let threads: Vec<_> = sends
            .iter()
            .map(|(session, bytes)| {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    session.accept(id, bytes)
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().expect("a request"))
            .collect()
    })
}

/// In each of five rounds, eve's acceptance through the first instance and frank's through a second
/// one, sent at once, leave one taken and the other refused, and the one taken is the one waiting.
#[test]
fn of_two_acceptances_sent_at_once_through_two_instances_one_is_kept() {
    let scene = Scene::new(3_817_300_301);
    let (eve, frank) = (&scene.f.eve, &scene.frank);
    let other = scene.team().flow.stand.instance(&[], &[]);

    for round in 0..5 {
        let invite = scene.invite();
        let sends = [
            (eve.session.clone(), acceptance(&invite, eve)),
            (frank.session.on(&other), acceptance(&invite, frank)),
        ];
        let answers = sent_together(invite.id, &sends);
        let statuses = [answers[0].status, answers[1].status];
        let (winner, loser) = match statuses {
            [200 | 201, _] => (0, 1),
            _ => (1, 0),
        };
        let what = format!(
            "round {round}: {}, then {}",
            text(&answers[0]),
            text(&answers[1])
        );
        assert!(matches!(statuses[winner], 200 | 201), "one taken: {what}");
        assert_refused(&answers[loser], &what);
        let waiting = scene.waiting_bytes();
        assert!(
            waiting.contains(&sends[winner].1),
            "the one taken waits: {what}"
        );
        assert!(
            !waiting.contains(&sends[loser].1),
            "the other does not: {what}"
        );
    }
}

/// An acceptance of an invite whose `expires` has passed is refused, and nothing waits.
#[test]
fn an_acceptance_after_expires_is_refused() {
    let scene = Scene::new(3_817_300_401);
    let eve = &scene.f.eve;
    let expires = now() + 4;
    let invite = scene.invite_until(expires);
    wait_past(expires);

    assert_refused(
        &eve.session.accept(invite.id, &acceptance(&invite, eve)),
        "after expires",
    );
    assert_eq!(scene.waiting_bytes(), Vec::<Vec<u8>>::new());
}
