//! Storing what member devices upload ("Uploads" and "History entry"): links, wraps and entries
//! are given back byte for byte, the same bytes again are stored once, an entry is kept per sending
//! device and `msg_id`, and the members of each epoch are those of the group the server follows,
//! on every instance.

use skald_proto::MessageId;

use crate::kit::{
    ENTRIES, LINKS, WRAPS, assert_given, assert_history, bob_rotates, entry, entry_of, formed,
    link, of_epoch_2, refuses, sealed_link, sealed_wrap, stores, wrap,
};

#[test]
fn links_wraps_and_entries_of_member_devices_are_given_back_byte_for_byte() {
    let (mut s, _) = formed(3_160_110_001);
    let epoch = bob_rotates(&mut s);
    assert_eq!(epoch, 2, "bob's empty Commit starts epoch 2");
    let f = &s.fixture;
    let t = &f.team;
    let general = f.general;
    // The keys of epochs 1 and 2, from MLS.
    let first = (s.bob.mls.history_key(general, 1)).expect("bob's key of epoch 1");
    let second = (s.bob.mls.history_key(general, 2)).expect("bob's key of epoch 2");

    let link = sealed_link(second, general, 2, 1, first);
    let alices = sealed_wrap(second, general, 2, &t.alice, &t.alice);
    let bobs = sealed_wrap(first, general, 1, &t.bob, &t.bob);
    let carols = entry(general, 2);
    let bobs_entry = entry(general, 1);
    stores(&mut s.bob, f, &link, "bob's link of epoch 2");
    stores(&mut s.alice, f, &alices, "alice's wrap of epoch 2");
    stores(&mut s.bob, f, &bobs, "bob's wrap of epoch 1");
    stores(&mut s.carol, f, &carols, "carol's entry of epoch 2");
    stores(
        &mut s.bob,
        f,
        &bobs_entry,
        "bob's entry of the previous epoch",
    );

    for peer in [&s.alice, &s.bob, &s.carol, &s.dave] {
        let given = [&[&link][..], &[&alices, &bobs], &[&carols, &bobs_entry]];
        assert_history(&peer.session, f, given, "a channel member's session");
    }
}

#[test]
fn the_same_bytes_uploaded_again_are_stored_once() {
    let (mut s, _) = formed(3_160_110_011);
    bob_rotates(&mut s);
    let f = &s.fixture;
    let t = &f.team;
    let [link, wrap, entry] = of_epoch_2(f, &t.carol, &t.bob);
    for (item, what) in [(&link, "a link"), (&wrap, "a wrap"), (&entry, "an entry")] {
        stores(&mut s.bob, f, item, what);
        stores(&mut s.bob, f, item, &format!("{what} again"));
    }
    // A link has no signer: another member device may upload the same bytes.
    stores(&mut s.alice, f, &link, "the same link from alice");

    assert_history(&s.carol.session, f, [&[&link], &[&wrap], &[&entry]], "once");
}

#[test]
fn entries_of_two_devices_with_one_msg_id_are_both_kept() {
    let (mut s, _) = formed(3_160_110_021);
    let f = &s.fixture;
    let msg_id = MessageId::generate();
    let bobs = entry_of(msg_id, f.general, 1);
    let carols = entry_of(msg_id, f.general, 1);
    stores(&mut s.bob, f, &bobs, "bob's entry");
    stores(&mut s.carol, f, &carols, "carol's entry with bob's msg_id");
    assert_given(
        &s.alice.session,
        f,
        ENTRIES,
        &[&bobs, &carols],
        "one per sending device",
    );
}

#[test]
fn another_instance_knows_the_member_devices_of_earlier_epochs() {
    let (mut s, _) = formed(3_160_110_031);
    s.add_dave();
    let other = s.fixture.team.flow.stand.instance(&[], &[]);
    let t = &s.fixture.team;
    s.bob.connect(&other, &s.fixture, &t.bob);
    s.dave.connect(&other, &s.fixture, &t.dave);
    let f = &s.fixture;
    let t = &f.team;

    // dave, added at epoch 2, was no member of epoch 1.
    let daves = wrap(f.general, 1, &t.alice, &t.dave);
    refuses(&mut s.dave, f, &daves, "dave's wrap of epoch 1");
    let bobs = wrap(f.general, 1, &t.alice, &t.bob);
    stores(&mut s.bob, f, &bobs, "bob's wrap of epoch 1");
    let link = link(f.general, 2, 1);
    stores(&mut s.dave, f, &link, "dave's link of epoch 2");

    let session = &s.dave.session;
    assert_given(session, f, WRAPS, &[&bobs], "through the other instance");
    assert_given(session, f, LINKS, &[&link], "through the other instance");
}
