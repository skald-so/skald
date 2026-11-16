//! Counters under `mls`: content with a counter already accepted is dropped, a missing one is
//! accepted, and each channel counts apart.

use crate::gaps::{assert_none_reported, bob_says, bob_says_in, bob_sends, sends};
use crate::kit::received;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "kickoff notes are in the shared folder";
const LATE: &str = "agenda attached, comments by friday";
const THIRD: &str = "room booked for the whole afternoon";
const REPEATED: &str = "the same counter once more";
const AGAIN: &str = "the late counter once more";
const ELSEWHERE: &str = "general chat: lunch at one";

/// Bob's device sends counters 1 and 3 in the channel, 2 held back; Ada's `skald recv` shows them.
/// Bob's device then sends, in order: new content with counter 3 again; the message with counter 2,
/// late; new content with counter 2 again; and counter 1 in the space's default channel. Ada's next
/// run drops both repeated counters, showing none of their text, shows the late message once, and
/// the default channel's message once, and reports nothing missing.
#[test]
fn counter_already_accepted_is_dropped_and_a_missing_one_accepted_per_channel() {
    let mut talk = Talk::new(
        &[4_137_072_001, 4_137_072_002],
        &["ada-counts-per-channel", "bob-repeats-himself"],
        false,
    );
    let from_bob = talk.address(&talk.played[0].person);
    let (channel, default) = (talk.channel, talk.space.default);
    bob_sends(&mut talk, 1, FIRST);
    let late = bob_says(&mut talk, 2, LATE);
    bob_sends(&mut talk, 3, THIRD);
    let first = received(&talk.ada.machine).shown();
    assert!(!first.contains(LATE), "the held back message: {first}");
    bob_sends(&mut talk, 3, REPEATED);
    sends(&talk, &late);
    bob_sends(&mut talk, 2, AGAIN);
    let elsewhere = bob_says_in(&mut talk, default, 1, ELSEWHERE);
    sends(&talk, &elsewhere);

    let shown = received(&talk.ada.machine).shown();

    for text in [REPEATED, AGAIN] {
        assert!(!shown.contains(text), "{text:?} shown: {shown}");
    }
    assert_shows_once(&shown, LATE, (channel, &from_bob, BOB_HOST));
    assert_shows_once(&shown, ELSEWHERE, (default, &from_bob, BOB_HOST));
    assert_none_reported(&shown);
}
