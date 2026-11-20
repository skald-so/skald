//! A device's value goes when a Commit the server accepts removes the device from the group, and
//! when the device is revoked; the other devices' values stay.

use crate::kit::{
    assert_all_kept, assert_given, assert_gone, assert_refused_with, callers, formed,
};
use crate::scene::{removing, revoke};

/// alice's, bob's and carol's laptops upload values; alice's Commit removing bob's laptop deletes
/// bob's value: alice and carol get theirs alone, and bob's laptop gets nothing.
#[test]
fn a_commit_that_removes_a_device_deletes_its_value() {
    let (mut s, general) = formed(3_621_300_001);
    let [alice, bob, carol, ..] = callers(&s);
    let values = [s.alice.id, s.bob.id, s.carol.id].map(|device| general.value(device));
    let [of_alice, of_bob, of_carol] = &values;
    assert_all_kept(
        &general,
        &[(&alice, of_alice), (&bob, of_bob), (&carol, of_carol)],
    );
    assert_given(&[&carol], &general, &[of_alice, of_bob, of_carol], "before");

    let commit = s.alice.commits(&s.fixture, removing(&[s.bob.id]));
    s.carol.process(&s.fixture, &commit);
    assert_gone(s.fixture.db(), of_bob, "bob's after his removal");
    assert_given(&[&alice, &carol], &general, &[of_alice, of_carol], "after");
    let response = bob.get(&general, true);
    assert_refused_with(&response, None, "bob's laptop after its removal");
}

/// bob's and carol's laptops upload values; the revocation of bob's laptop deletes bob's value,
/// and alice gets carol's alone.
#[test]
fn a_revoked_devices_value_is_deleted() {
    let (s, general) = formed(3_621_300_011);
    let [alice, bob, carol, ..] = callers(&s);
    let of_bob = general.value(s.bob.id);
    let of_carol = general.value(s.carol.id);
    assert_all_kept(&general, &[(&bob, &of_bob), (&carol, &of_carol)]);
    assert_given(&[&alice], &general, &[&of_bob, &of_carol], "before");

    revoke(&s.fixture.team.bob);
    assert_gone(s.fixture.db(), &of_bob, "bob's after the revocation");
    assert_given(&[&alice], &general, &[&of_carol], "after");
}
