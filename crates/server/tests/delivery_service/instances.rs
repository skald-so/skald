//! The group the server follows, in its database: across instances on one database, of Commits of
//! one epoch sent at once the first stored wins, the channel's envelopes are in one order in every
//! member's mailbox, and each instance and a restart go on from the group as the others left it.

use std::sync::Barrier;
use std::thread;

use crate::common::Server;
use crate::scene::{
    EPOCH_MISMATCH, Peer, Scene, assert_latest, assert_upload_refused, assert_uploaded, empty,
    latest_of, mls_envelope, mls_header, payloads, upload,
};
use crate::wire::{Answer, NOT_A_MEMBER, send_frame};

/// Another instance of the scene's server on its database.
fn instance(s: &Scene) -> Server {
    s.fixture.team.flow.stand.instance(&[], &[])
}

/// Sends each of `sends`, a peer and its envelopes, at once, each peer on its own thread, all its
/// envelopes before reading an answer; gives each peer's answers, in the order of `sends`.
fn at_once(sends: Vec<(&mut Peer, Vec<Vec<u8>>)>) -> Vec<Vec<Answer>> {
    let start = Barrier::new(sends.len());
    thread::scope(|scope| {
        let sending: Vec<_> = (sends.into_iter())
            .map(|(peer, envelopes)| {
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    for envelope in &envelopes {
                        peer.socket.frame(send_frame(envelope, false));
                    }
                    peer.socket.answers(envelopes.len())
                })
            })
            .collect();
        (sending.into_iter())
            .map(|sender| sender.join().expect("a sender"))
            .collect()
    })
}

/// The problem of `answer`, none if stored.
fn problem(answer: &Answer) -> Option<&str> {
    match answer {
        Answer::Stored(_) => None,
        Answer::Refused(_, problem) => Some(problem),
    }
}

/// Sends `alices` on alice's connection and `bobs` on bob's at once, each in an envelope to the
/// channel; gives which the server stored, 0 for alice's and 1 for bob's, checking that it refused
/// the other with `epoch-mismatch`.
#[track_caller]
fn one_stored(s: &mut Scene, alices: &[u8], bobs: &[u8], what: &str) -> usize {
    let f = &s.fixture;
    let [to_alice, to_bob] = [alices, bobs].map(|commit| mls_envelope(&mls_header(f, &[]), commit));
    let answers = at_once(vec![
        (&mut s.alice, vec![to_alice]),
        (&mut s.bob, vec![to_bob]),
    ]);
    let problems: Vec<Option<&str>> = answers.iter().map(|answers| problem(&answers[0])).collect();
    match problems.as_slice() {
        [None, Some(EPOCH_MISMATCH)] => 0,
        [Some(EPOCH_MISMATCH), None] => 1,
        other => panic!("{what}: one stored and one epoch-mismatch, not {other:?}"),
    }
}

#[test]
fn of_two_commits_of_one_epoch_sent_at_once_through_two_instances_one_is_stored() {
    let (mut s, formed) = Scene::formed(3_121_470_401);
    let other = instance(&s);
    s.bob.connect(&other, &s.fixture, &s.fixture.team.bob);
    let mut stored = vec![formed.welcome.clone()];
    for round in 0..5 {
        let general = s.fixture.general;
        let [alices, bobs] = [&mut s.alice, &mut s.bob]
            .map(|peer| empty(&mut peer.mls, general).expect("an empty Commit"));
        let winner = one_stored(&mut s, &alices, &bobs, &format!("round {round}"));
        let commits = [alices, bobs];
        let f = &s.fixture;
        let (won, lost) = if winner == 0 {
            (&mut s.alice, &mut s.bob)
        } else {
            (&mut s.bob, &mut s.alice)
        };
        let (_, group_info) = won.merges_and_uploads(f);
        lost.process(f, &commits[winner]);
        s.carol.process(f, &commits[winner]);
        assert_latest(&s.carol.session, f, &group_info, &format!("round {round}"));
        stored.push(commits[winner].clone());
    }
    let held = payloads(s.fixture.db(), s.carol.id);
    assert_eq!(held, stored, "carol's mailbox: each stored Commit");
    assert_eq!(s.carol.mls.epoch(s.fixture.general).expect("the group"), 6);
}

#[test]
fn of_two_first_commits_sent_at_once_through_two_instances_one_is_kept() {
    let mut s = Scene::new(3_121_470_411);
    let other = instance(&s);
    s.bob.connect(&other, &s.fixture, &s.fixture.team.bob);
    let alices = s.creates();
    let packages = [s.alice.key_package(), s.carol.key_package()];
    let admitted = s.admitted();
    let bobs = s.bob.creates(&s.fixture, &packages, &admitted);
    let winner = one_stored(&mut s, &alices, &bobs, "the first Commits");
    let f = &s.fixture;
    let (won, lost, joiner) = if winner == 0 {
        (&mut s.alice, &mut s.bob, &mut s.carol)
    } else {
        (&mut s.bob, &mut s.alice, &mut s.carol)
    };

    let mut fork = lost.copy();
    fork.commit_stored(f.general).expect("the fork merges");
    let forked = fork.group_info(f.general).expect("the fork's GroupInfo");
    assert_upload_refused(&lost.session, f, &forked, "the refused creator's");
    let (welcome, group_info) = won.merges_and_uploads(f);
    let welcome = welcome.expect("a Welcome");
    assert_upload_refused(&lost.session, f, &forked, "the refused creator's, then");

    lost.mls.commit_refused(f.general).expect("drops its group");
    won.stores(f, &welcome, &[lost.id, joiner.id]);
    for peer in [lost, joiner] {
        peer.joins(f, &welcome);
    }
    assert_latest(&s.carol.session, f, &group_info, "the kept group's");
}

#[test]
fn envelopes_sent_at_once_through_two_instances_are_in_one_order_in_every_mailbox() {
    let (mut s, _) = Scene::formed(3_121_470_421);
    s.add_dave();
    let other = instance(&s);
    s.bob.connect(&other, &s.fixture, &s.fixture.team.bob);
    let f = &s.fixture;
    let messages = |peer: &mut Peer| -> Vec<Vec<u8>> {
        (0..20)
            .map(|at| (peer.mls.encrypt(f.general, format!("{at}").as_bytes())).expect("a message"))
            .collect()
    };
    let (from_alice, from_bob) = (messages(&mut s.alice), messages(&mut s.bob));
    let envelopes = |payloads: &[Vec<u8>]| -> Vec<Vec<u8>> {
        (payloads.iter())
            .map(|payload| mls_envelope(&mls_header(f, &[]), payload))
            .collect()
    };
    let answers = at_once(vec![
        (&mut s.alice, envelopes(&from_alice)),
        (&mut s.bob, envelopes(&from_bob)),
    ]);
    for answer in answers.iter().flatten() {
        assert_eq!(problem(answer), None, "stored");
    }

    let db = f.db();
    let of = |held: Vec<Vec<u8>>, sent: &[Vec<u8>]| -> Vec<Vec<u8>> {
        held.into_iter()
            .filter(|payload| sent.contains(payload))
            .collect()
    };
    let all = [from_alice.clone(), from_bob.clone()].concat();
    let carols = of(payloads(db, s.carol.id), &all);
    assert_eq!(carols.len(), 40, "each message once");
    assert_eq!(
        of(payloads(db, s.dave.id), &all),
        carols,
        "dave's in carol's order"
    );
    assert_eq!(
        of(payloads(db, s.alice.id), &from_bob),
        of(carols.clone(), &from_bob)
    );
    assert_eq!(
        of(payloads(db, s.bob.id), &from_alice),
        of(carols.clone(), &from_alice)
    );
    // Each sender's in the order it sent them.
    assert_eq!(of(carols.clone(), &from_alice), from_alice);
    assert_eq!(of(carols, &from_bob), from_bob);
}

#[test]
fn another_instance_goes_on_from_the_group_one_instance_followed() {
    let (mut s, formed) = Scene::formed(3_121_470_431);
    let other = instance(&s);
    s.bob.connect(&other, &s.fixture, &s.fixture.team.bob);
    let f = &s.fixture;
    let carols = empty(&mut s.carol.copy(), f.general).expect("carol's Commit of epoch 1");
    // bob's Commit through the other instance; carol's of the same epoch through the first.
    let bobs = s.bob.commits(f, empty);
    s.carol
        .assert_refused(f, &carols, &[], EPOCH_MISMATCH, "carol's of epoch 1");
    s.bob.assert_refused(
        f,
        &bobs,
        &[],
        EPOCH_MISMATCH,
        "bob's again, through the other instance",
    );
    let latest = s.bob.group_info(f);
    assert_latest(
        &s.carol.session,
        f,
        &latest,
        "bob's, through the first instance",
    );
    assert_upload_refused(
        &s.alice.session,
        f,
        &formed.group_info,
        "alice's of epoch 1",
    );

    // carol's of epoch 2 through the first instance; its GroupInfo through the other one.
    s.carol.process(f, &bobs);
    let commit = empty(&mut s.carol.mls, f.general).expect("an empty Commit");
    s.carol.stores(f, &commit, &[]);
    s.carol.mls.commit_stored(f.general).expect("it merges");
    let carols = s.carol.group_info(f);
    let on_other = s.carol.session.on(&other);
    assert_uploaded(
        &upload(&on_other, f, &carols),
        "carol's, through the other instance",
    );
    assert_latest(&s.alice.session, f, &carols, "carol's, through the first");
}

#[test]
fn after_a_restart_the_server_goes_on_from_the_group_it_followed() {
    let (mut s, formed) = Scene::formed(3_121_470_441);
    let f = &s.fixture;
    let bobs = s.bob.commits(f, empty);
    let latest = s.bob.group_info(f);
    for peer in [&mut s.alice, &mut s.carol] {
        peer.process(f, &bobs);
    }

    let stand = &mut s.fixture.team.flow.stand;
    stand.server.stop();
    let server = stand.instance(&[], &[]);
    let t = &s.fixture.team;
    for (peer, person) in [
        (&mut s.alice, &t.alice),
        (&mut s.bob, &t.bob),
        (&mut s.carol, &t.carol),
        (&mut s.dave, &t.dave),
    ] {
        peer.connect(&server, &s.fixture, person);
    }

    let f = &s.fixture;
    assert_latest(&s.dave.session, f, &latest, "bob's, after the restart");
    s.bob
        .assert_refused(f, &bobs, &[], EPOCH_MISMATCH, "bob's Commit again");
    s.alice.assert_refused(
        f,
        &formed.commit,
        &[],
        EPOCH_MISMATCH,
        "the first Commit again",
    );
    let carols = empty(&mut s.carol.mls, f.general).expect("an empty Commit");
    s.alice
        .assert_refused(f, &carols, &[], NOT_A_MEMBER, "carol's, sent by alice");
    s.carol.stores(f, &carols, &[]);
    assert_eq!(latest_of(&s.dave.session, f), (true, None), "a new epoch");
}
