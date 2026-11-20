//! Values the server keeps: one per channel and device, the latest a member device of the current
//! epoch uploaded, given back to the channel's member devices byte for byte.

use crate::kit::{
    LONGEST_CT, assert_all_kept, assert_given, assert_kept, assert_not_held, assert_refused_with,
    callers, epoch, formed,
};
use crate::scene::EPOCH_MISMATCH;
use crate::wire::NOT_A_MEMBER;

/// alice's, bob's and carol's laptops, the group's members, each upload a value; each of them
/// gets the three, byte for byte, one of them with a field of a later version and one with a `ct`
/// of the longest length the server takes. alice's next value replaces her first: each member
/// gets it in its place, and the server keeps nothing of the first.
#[test]
fn a_member_devices_latest_value_is_given_back_byte_for_byte() {
    let (s, general) = formed(3_621_100_001);
    let [alice, bob, carol, ..] = callers(&s);
    let mut first = general.value(s.alice.id);
    first.later.push((6, b"\x63new".to_vec()));
    let of_bob = general.value(s.bob.id).with_ct(LONGEST_CT);
    let of_carol = general.value(s.carol.id);
    assert_all_kept(
        &general,
        &[(&alice, &first), (&bob, &of_bob), (&carol, &of_carol)],
    );
    let members = [&alice, &bob, &carol];
    assert_given(
        &members,
        &general,
        &[&first, &of_bob, &of_carol],
        "the three",
    );

    let next = general.value(s.alice.id);
    assert_kept(&alice.put(&general, &next), "alice's next");
    let latest = [&next, &of_bob, &of_carol];
    assert_given(&members, &general, &latest, "after alice's next");
    assert_not_held(s.fixture.db(), &first, "alice's first");
}

/// dave's laptop, a channel member outside the group, is refused with `not-a-member`; once alice's
/// Commit adds it, its value of the new epoch is kept, and so is alice's, while hers of the epoch
/// before is refused with `epoch-mismatch`. bob, carol and dave get both new values.
#[test]
fn a_device_a_commit_adds_uploads_in_its_epoch() {
    let (mut s, before) = formed(3_621_100_011);
    let [alice, bob, carol, dave, _] = callers(&s);
    let early = before.value(s.dave.id);
    let response = dave.put(&before, &early);
    assert_refused_with(&response, Some(NOT_A_MEMBER), "dave before");
    assert_not_held(s.fixture.db(), &early, "dave's before");

    s.add_dave();
    let after = before.at(epoch(&s));
    let of_dave = after.value(s.dave.id);
    assert_kept(&dave.put(&after, &of_dave), "dave's in the new epoch");
    let stale = before.value(s.alice.id);
    let response = alice.put(&before, &stale);
    assert_refused_with(
        &response,
        Some(EPOCH_MISMATCH),
        "alice's of the epoch before",
    );
    let of_alice = after.value(s.alice.id);
    assert_kept(&alice.put(&after, &of_alice), "alice's in the new epoch");

    let expected = [&of_alice, &of_dave];
    assert_given(
        &[&bob, &carol, &dave],
        &after,
        &expected,
        "after the Commit",
    );
}
