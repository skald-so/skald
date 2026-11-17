//! The first upload kept ("Uploads" and "History entry"): the first link per epoch and `prev`, the
//! first wrap per epoch and recipient, and the first entry per sending device and `msg_id`; a later
//! one changes nothing.

use crate::kit::{
    ENTRIES, LINKS, WRAPS, answers, assert_given, bob_rotates, entry_of, formed, link, stores, wrap,
};
use crate::wire::assert_nowhere;

#[test]
fn of_two_links_with_one_epoch_and_prev_the_first_is_kept_and_one_with_another_prev_beside_it() {
    let (mut s, _) = formed(3_160_110_101);
    bob_rotates(&mut s);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let first = link(f.general, 3, 2);
    let later = link(f.general, 3, 2);
    let other_prev = link(f.general, 3, 1);
    stores(&mut s.alice, f, &first, "alice's link of epoch 3 to 2");
    answers(&mut s.bob, f, &later);
    stores(&mut s.carol, f, &other_prev, "carol's link of epoch 3 to 1");
    answers(&mut s.alice, f, &later);

    assert_given(
        &s.dave.session,
        f,
        LINKS,
        &[&first, &other_prev],
        "the first per epoch and prev",
    );
    assert_nowhere(f.db(), later.bytes());
}

#[test]
fn of_two_wraps_to_one_recipient_in_one_epoch_the_first_is_kept() {
    let (mut s, _) = formed(3_160_110_111);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let first = wrap(f.general, 2, &t.dave, &t.alice);
    let later = wrap(f.general, 2, &t.dave, &t.bob);
    let other_recipient = wrap(f.general, 2, &t.carol, &t.bob);
    let other_epoch = wrap(f.general, 1, &t.dave, &t.bob);
    stores(&mut s.alice, f, &first, "alice's wrap of epoch 2 to dave");
    answers(&mut s.bob, f, &later);
    stores(&mut s.bob, f, &other_recipient, "bob's to carol");
    stores(&mut s.bob, f, &other_epoch, "bob's of epoch 1 to dave");

    assert_given(
        &s.dave.session,
        f,
        WRAPS,
        &[&first, &other_recipient, &other_epoch],
        "the first per epoch and recipient",
    );
    assert_nowhere(f.db(), later.bytes());
}

#[test]
fn a_retried_entry_with_other_bytes_leaves_the_first() {
    let (mut s, _) = formed(3_160_110_121);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let msg_id = skald_proto::MessageId::generate();
    let first = entry_of(msg_id, f.general, 2);
    let retried = entry_of(msg_id, f.general, 2);
    let previous_epoch = entry_of(msg_id, f.general, 1);
    stores(&mut s.bob, f, &first, "bob's entry");
    answers(&mut s.bob, f, &retried);
    answers(&mut s.bob, f, &previous_epoch);

    assert_given(
        &s.alice.session,
        f,
        ENTRIES,
        &[&first],
        "the first per sending device and msg_id",
    );
    for item in [&retried, &previous_epoch] {
        assert_nowhere(f.db(), item.bytes());
    }
}
