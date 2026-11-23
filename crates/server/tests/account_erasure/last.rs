//! What the spaces an erased account shared keep of it goes with the last such space.

use skald_proto::Role;

use crate::api::Team;
use crate::deletion::{acme_with_member, deletes, with_role};
use crate::erasure::{ERASED_AFTER, Kept, Traces, erased_within, move_deletion_back};
use crate::rosters::next;
use crate::sessions::Api;

/// In `acme`, alice and dave are admins and no one else is a member. alice deletes her account, and
/// 30 days later it is erased but for what `acme` keeps. Once dave, its last other member, deletes
/// his account too, within the erasure's time, no row of the database holds anything of hers.
#[test]
fn nothing_of_the_account_stays_once_the_last_space_it_shared_goes() {
    let github = 3_304_040_001;
    let team = Team::new(github);
    let (alice, dave) = (&team.alice, &team.dave);
    let [_, two] = acme_with_member(&team, dave);
    let mut body = next(&two, &alice.laptop);
    with_role(&mut body, dave, Role::Admin);
    team.stored_by(alice, &alice.sign(&body));
    let devices = [alice.laptop.id, alice.uncertified.id];
    let traces = Traces::of(alice, github, &devices);
    let kept = Kept::of_space(&dave.session, team.space, &devices);
    let api = Api::of(&team.flow);
    let db = &team.flow.stand.db;
    deletes(&api, alice);
    move_deletion_back(db, alice.account, ERASED_AFTER);
    erased_within(db, &traces, &kept);

    deletes(&api, dave);

    erased_within(db, &traces, &Kept::nothing());
}
