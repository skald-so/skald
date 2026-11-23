//! A roster revision and the erasure of the space's creator can commit together.

use std::thread;

use skald_proto::Role;

use crate::api::Team;
use crate::common::devices::assert_taken;
use crate::deletion::deletes;
use crate::erasure::{ERASED_AFTER, Kept, Traces, erased_within, move_deletion_back};
use crate::held::{Held, until_waiting};
use crate::rosters::{first, member, next};
use crate::sessions::Api;

/// alice's revision holds the space's lock while it waits on her account. dave's erasure holds
/// his account's lock while it waits on the space. Once alice's lock is released, the revision
/// can take KEY SHARE on dave and commit, and the erasure can clear the creator.
#[test]
fn erasure_and_a_roster_revision_commit_without_deadlock() {
    let github = 3_304_050_031;
    let mut team = Team::new(github);
    let (alice, dave) = (&team.alice, &team.dave);
    let db = &team.flow.stand.db;
    let name = "shared";
    let space = dave.create_space(name);
    let one = dave.sign(&first(space, name, dave));
    assert_taken(&dave.session.store(space, &one));
    let mut body = next(&one, &dave.laptop);
    body.members.push(member(alice, Role::Admin));
    let two = dave.sign(&body);
    assert_taken(&dave.session.store(space, &two));
    let three = alice.sign(&next(&two, &alice.laptop));
    let devices = [dave.laptop.id, dave.uncertified.id];
    let traces = Traces::of(dave, github + 3, &devices);
    deletes(&Api::of(&team.flow), dave);

    // A revision locks the accounts in ID order. alice signed up before dave.
    assert!(alice.account.as_bytes() < dave.account.as_bytes());
    thread::scope(|scope| {
        let held = Held::account(db, alice.account);
        let storing = scope.spawn(|| alice.session.store(space, &three));
        until_waiting(db, 1, "the revision");
        move_deletion_back(db, dave.account, ERASED_AFTER);
        until_waiting(db, 2, "the erasure");
        held.release();
        assert_taken(&storing.join().expect("the revision"));
    });

    let kept = Kept::of_space(&alice.session, space, &devices);
    erased_within(db, &traces, &kept);
    let output = team.flow.stand.server.stop();
    assert!(
        !output.contains("cannot erase deleted accounts:"),
        "{output}"
    );
}
