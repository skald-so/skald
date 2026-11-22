//! Before its deletion is 30 days old, an account loses nothing more than the deletion took at once.

use crate::deletion::deletes;
use crate::erasure::{
    ERASED_AFTER, Kept, NOT_YET, Traces, erased_within, move_deletion_back, rows_with,
};
use crate::fixture::Fixture;
use crate::sessions::Api;
use crate::shared::alices_records;

/// alice keeps the records of the shared scenario in `acme`, where bob is an admin too; eve is in
/// no space of another account. Both delete their accounts; alice's deletion is moved back 29 days
/// and 23 hours, eve's 30 days. Once eve's account is erased, every row that held alice's data
/// right after the move still holds it.
#[test]
fn before_thirty_days_nothing_more_than_the_deletion_took_is_gone() {
    let github = 3_304_030_001;
    let mut f = Fixture::new(github);
    let (phone, _) = alices_records(&mut f);
    let t = &f.team;
    let alice = Traces::of(
        &t.alice,
        github,
        &[t.alice.laptop.id, t.alice.uncertified.id, phone.id],
    );
    let eve = Traces::of(&f.eve, github + 4, &[f.eve.laptop.id, f.eve.uncertified.id]);
    let api = Api::of(&t.flow);
    deletes(&api, &t.alice);
    deletes(&api, &f.eve);
    move_deletion_back(f.db(), t.alice.account, NOT_YET);
    move_deletion_back(f.db(), f.eve.account, ERASED_AFTER);
    let held = rows_with(f.db(), &alice);

    erased_within(f.db(), &eve, &Kept::mark());

    let now = rows_with(f.db(), &alice);
    let gone: Vec<&String> = held.difference(&now).collect();
    assert!(
        gone.is_empty(),
        "alice's rows gone before 30 days: {gone:#?}"
    );
}
