//! A CLI killed after writing its entries: what it keeps of the counters is written with them.

use crate::fixtures::killed_once;
use crate::front::Acks;
use crate::gaps::{
    assert_none_reported, assert_reported_once, bob_in, bob_sends, marked, second_dropped,
};
use crate::kit::{RECV, received};
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// The texts the tests send; none is anywhere else.
const SAYS: [&str; 4] = [
    "deploy window opens at six",
    "deploy paused for a hotfix",
    "deploy resumed, all green",
    "deploy finished, closing the window",
];

/// Bob's device sends three messages to the channel; the second never reaches the server. Ada's
/// `skald recv` is killed once it has written them and sent its ack, which the front swallowed,
/// before catch-up ended. Ada's next run, which gets the entries again, and the killed one show the
/// first and the third once in all, and report messages missing once in all. Bob's device sends a
/// fourth: Ada's next run shows it unmarked and reports nothing missing.
#[test]
fn cli_killed_after_writing_neither_shows_a_message_again_nor_reports_one_missing() {
    let mut talk = Talk::new(
        &[4_137_075_001, 4_137_075_002],
        &["ada-killed-mid-gap", "bob-steady-sender"],
        false,
    );
    let (from_bob, channel, bob) = bob_in(&talk);
    second_dropped(&mut talk, [SAYS[0], SAYS[1], SAYS[2]]);
    let acked = talk.front.acked();
    talk.front.acks(Acks::Swallow);
    talk.front.hold_end(true);

    let front = &talk.front;
    let killed = killed_once(talk.ada.machine.start(&[RECV]), "ack", || {
        front.acked() > acked
    });
    talk.front.acks(Acks::Forward);
    talk.front.hold_end(false);
    let next = received(&talk.ada.machine);

    let shown = format!("{}\n{}", killed.shown(), next.shown());
    for text in [SAYS[0], SAYS[2]] {
        assert_shows_once(&shown, text, (channel, &from_bob, BOB_HOST));
    }
    assert_reported_once(&shown, channel, bob);
    let fourth = bob_sends(&mut talk, 4, SAYS[3]);

    let later = received(&talk.ada.machine).shown();

    assert_shows_once(&later, SAYS[3], (channel, &from_bob, BOB_HOST));
    assert!(
        !marked(&later, fourth),
        "the fourth message marked: {later}"
    );
    assert_none_reported(&later);
}
