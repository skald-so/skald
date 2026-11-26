//! A space that removed the account before its deletion: the deletion leaves the rows of its
//! devices' long-polls of that space, and the erasure deletes them.

use skald_proto::{NonEmpty, Role};

use crate::api::Team;
use crate::deletion::{acme_with_member, deletes, with_role};
use crate::erasure::{ERASED_AFTER, Kept, Traces, erased_within, move_deletion_back};
use crate::https::Https;
use crate::rosters::next;
use crate::sessions::Api;

/// In `acme`, alice's laptop long-polls, which records the revision of `acme` it answered; alice
/// then leaves `acme` to dave, its new admin. Once she deletes her account and 30 days pass, no row
/// holds her data beyond what `acme` keeps, the long-poll's row among them.
#[test]
fn erases_the_long_polls_of_a_space_that_removed_it() {
    let github = 3_304_050_021;
    let team = Team::new(github);
    let (alice, dave) = (&team.alice, &team.dave);
    let db = &team.flow.stand.db;
    let [_, two] = acme_with_member(&team, dave);
    Https::of(&team.flow, alice).poll_now(0);
    let mut body = next(&two, &alice.laptop);
    with_role(&mut body, dave, Role::Admin);
    let mut members = body.members.clone().into_vec();
    members.retain(|m| m.account != alice.account);
    body.members = NonEmpty::from_vec(members).expect("a member");
    team.stored_by(alice, &alice.sign(&body));
    let devices = [alice.laptop.id, alice.uncertified.id];
    let kept = Kept::of_space(&dave.session, team.space, &devices);
    let traces = Traces::of(alice, github, &devices);

    deletes(&Api::of(&team.flow), alice);
    move_deletion_back(db, alice.account, ERASED_AFTER);

    erased_within(db, &traces, &kept);
}
