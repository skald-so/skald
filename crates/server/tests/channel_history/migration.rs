//! A server that followed a channel's group before it kept history: the migration that adds it
//! takes the member devices of the group's current epoch, so they upload its links, wraps and
//! entries before the next Commit.

use crate::kit::{assert_history, bob_rotates, entry, formed, link, refuses, stores, wrap};

/// The migration that adds channels' history.
const CHANNEL_HISTORY: i64 = 20261215000000;

#[test]
fn migration_keeps_the_member_devices_of_each_followed_groups_current_epoch() {
    let (mut s, _) = formed(3_160_110_301);
    bob_rotates(&mut s);
    let f = &s.fixture;
    // The database as the server before history left it, with the group followed at epoch 2 of
    // alice, bob and carol; then migrated.
    f.db().execute(&format!(
        "DROP TABLE history_links, history_wraps, history_entries, channel_epochs;
         DELETE FROM _sqlx_migrations WHERE version = {CHANNEL_HISTORY};"
    ));
    f.db().migrate_to(CHANNEL_HISTORY);
    let t = &f.team;
    let g = f.general;

    let alices = link(g, 2, 1);
    stores(&mut s.alice, f, &alices, "alice's link of epoch 2");
    let bobs = wrap(g, 2, &t.dave, &t.bob);
    stores(&mut s.bob, f, &bobs, "bob's wrap of epoch 2");
    let carols = entry(g, 2);
    stores(&mut s.carol, f, &carols, "carol's entry of epoch 2");
    let daves = wrap(g, 2, &t.alice, &t.dave);
    refuses(&mut s.dave, f, &daves, "dave's, outside the group");
    let given = [&[&alices][..], &[&bobs], &[&carols]];
    assert_history(&s.dave.session, f, given, "the members' of epoch 2");
}
