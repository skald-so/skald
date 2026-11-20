//! Who gets a channel's values: its member devices, proving their keys; no device outside the
//! group and no session alone.

use crate::kit::{
    assert_given, assert_kept, assert_refused_with, assert_unproved, callers, formed,
};

/// alice's laptop uploads a value, which bob's and carol's get; eve's laptop, outside the space,
/// and dave's, a channel member outside the group, get nothing.
#[test]
fn a_device_outside_the_group_gets_nothing() {
    let (s, general) = formed(3_621_400_001);
    let [alice, bob, carol, dave, eve] = callers(&s);
    let of_alice = general.value(s.alice.id);
    assert_kept(&alice.put(&general, &of_alice), "alice's");
    assert_given(&[&bob, &carol], &general, &[&of_alice], "members");

    for caller in [&eve, &dave] {
        assert_refused_with(&caller.get(&general, true), None, &caller.who);
    }
}

/// bob's session without his laptop's proof gets nothing: `401` with `auth-required`. With the
/// proof, it gets alice's value.
#[test]
fn a_session_that_does_not_prove_its_device_gets_nothing() {
    let (s, general) = formed(3_621_400_011);
    let [alice, bob, ..] = callers(&s);
    let of_alice = general.value(s.alice.id);
    assert_kept(&alice.put(&general, &of_alice), "alice's");

    assert_unproved(&bob.get(&general, false), "bob's session alone");
    assert_given(&[&bob], &general, &[&of_alice], "with the proof");
}
