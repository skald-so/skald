//! The baseline under `mls` again: a device that joins a channel's group again counts from the
//! first message it gets from each sending device after that.

use crate::gaps::{assert_none_reported, bob_in, bob_sends, marked};
use crate::kit::received;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// The texts the test sends; none is anywhere else.
const BEFORE: [&str; 2] = [
    "inventory count starts monday",
    "inventory count is on aisle four",
];
const WITHOUT: [&str; 2] = ["aisle five has a recount", "recount done, two boxes short"];
const AFTER: &str = "inventory closed, report on friday";

/// Bob's device sends two messages to the channel, which Ada's `skald recv` shows. It then removes
/// Ada's device from the channel's group, sends two messages without it, adds it again and sends it
/// the Welcome, then sends a fifth. Ada's next run shows the fifth, unmarked, none of the two
/// without it, and reports nothing missing.
#[test]
fn device_that_joins_the_channel_again_reports_none_of_the_messages_before_missing() {
    let mut talk = Talk::new(
        &[4_137_077_001, 4_137_077_002],
        &["ada-joins-again", "bob-removes-and-adds"],
        false,
    );
    let (from_bob, channel, _) = bob_in(&talk);
    for (counter, text) in (1..).zip(BEFORE) {
        bob_sends(&mut talk, counter, text);
    }
    let shown = received(&talk.ada.machine).shown();
    for text in BEFORE {
        assert_shows_once(&shown, text, (channel, &from_bob, BOB_HOST));
    }
    let at = talk.at();
    (talk.played[0]).removes(&talk.world.cloud, at, talk.ada.device);
    for (counter, text) in (3..).zip(WITHOUT) {
        bob_sends(&mut talk, counter, text);
    }
    (talk.played[0]).welcomes(&talk.world.cloud, at, &talk.ada);
    let fifth = bob_sends(&mut talk, 5, AFTER);

    let next = received(&talk.ada.machine).shown();

    assert_shows_once(&next, AFTER, (channel, &from_bob, BOB_HOST));
    for text in WITHOUT {
        assert!(!next.contains(text), "{text:?} shown: {next}");
    }
    assert!(
        !marked(&next, fifth),
        "the first message after joining again marked: {next}"
    );
    assert_none_reported(&next);
}
