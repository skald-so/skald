//! Missing messages under `mls`: a message the server drops is reported once a later message from
//! the same device arrives, and the later message is marked.

use crate::gaps::{
    assert_none_reported, assert_reported_once, bob_in, bob_says, bob_sends, marked, second_dropped,
};
use crate::kit::received;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// The texts the tests send; none is anywhere else.
const SAYS: [&str; 4] = [
    "the freezer in lab two is warming up",
    "moved the samples to the backup freezer",
    "backup freezer holds at minus eighty",
    "technician comes tomorrow at nine",
];

/// Bob's device sends three messages to the channel; the second never reaches the server. Ada's
/// `skald recv` shows the first and the third once each, with the channel, Bob's address and his
/// device's name, the third marked as following missing ones and the first not, and reports
/// messages missing once, naming the channel and Bob's device. Bob's device sends a fourth: Ada's
/// next run shows it unmarked and reports nothing missing.
#[test]
fn message_dropped_between_two_is_reported_once_and_the_next_message_marked() {
    let mut talk = Talk::new(
        &[4_137_071_001, 4_137_071_002],
        &["ada-notices-gap", "bob-loses-second"],
        false,
    );
    let (from_bob, channel, bob) = bob_in(&talk);
    let (first, third) = second_dropped(&mut talk, [SAYS[0], SAYS[1], SAYS[2]]);

    let shown = received(&talk.ada.machine).shown();

    for text in [SAYS[0], SAYS[2]] {
        assert_shows_once(&shown, text, (channel, &from_bob, BOB_HOST));
    }
    assert!(!shown.contains(SAYS[1]), "the dropped message: {shown}");
    assert!(!marked(&shown, first), "the first message marked: {shown}");
    assert!(
        marked(&shown, third),
        "the third message not marked: {shown}"
    );
    assert_reported_once(&shown, channel, bob);
    let fourth = bob_sends(&mut talk, 4, SAYS[3]);

    let next = received(&talk.ada.machine).shown();

    assert_shows_once(&next, SAYS[3], (channel, &from_bob, BOB_HOST));
    assert!(!marked(&next, fourth), "the fourth message marked: {next}");
    assert_none_reported(&next);
}

/// Bob's device sends a message, then one that never reaches the server. Ada's `skald recv` shows
/// the first and reports nothing missing. Bob's device sends a third: Ada's next run shows it
/// marked, and reports messages missing once, naming the channel and Bob's device.
#[test]
fn latest_message_dropped_is_reported_only_once_the_next_one_arrives() {
    let mut talk = Talk::new(
        &[4_137_071_101, 4_137_071_102],
        &["ada-waits-for-next", "bob-loses-latest"],
        false,
    );
    let (from_bob, channel, bob) = bob_in(&talk);
    let first = bob_sends(&mut talk, 1, SAYS[0]);
    bob_says(&mut talk, 2, SAYS[1]);

    let shown = received(&talk.ada.machine).shown();

    assert_shows_once(&shown, SAYS[0], (channel, &from_bob, BOB_HOST));
    assert!(!marked(&shown, first), "the first message marked: {shown}");
    assert_none_reported(&shown);
    let third = bob_sends(&mut talk, 3, SAYS[2]);

    let next = received(&talk.ada.machine).shown();

    assert_shows_once(&next, SAYS[2], (channel, &from_bob, BOB_HOST));
    assert!(!next.contains(SAYS[1]), "the dropped message: {next}");
    assert!(marked(&next, third), "the third message not marked: {next}");
    assert_reported_once(&next, channel, bob);
}
