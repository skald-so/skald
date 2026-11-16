//! Gap detection under `hpke`: the counters Bob's device keeps for Ada's device, from 0, apart from
//! its `mls` counters in the channel.

use crate::fixtures::cert_of;
use crate::gaps::{assert_none_reported, assert_reported_once, bob_sends, marked, sends};
use crate::kit::received;
use crate::talk::{BOB_HOST, Line, Talk, assert_shows_once, sealed};

/// The texts the tests send; none is anywhere else.
const OVER_MLS: [&str; 2] = [
    "grouped hello before the sealed ones",
    "grouped goodbye before the sealed ones",
];
const HELD: &str = "sealed note that comes late";
const SECOND: &str = "sealed note with counter two";
const REPEATED: &str = "sealed note reusing counter two";

/// Bob's device sends `mls` counters 1 and 2 in the channel, which Ada's `skald recv` shows. Then,
/// sealed to Ada's device under `hpke`, counter 1 is held back and counter 2 sent: Ada's next run
/// shows it marked and reports messages missing once, naming the channel and Bob's device. Bob's
/// device then sends new content with counter 2 again and the held back counter 1, which the front
/// also repeats under another `seq`: Ada's next run drops the repeated counter, showing none of its
/// text, shows the late message once, and reports nothing missing.
#[test]
fn dropped_hpke_message_is_reported_and_a_repeated_counter_dropped() {
    let mut talk = Talk::new(
        &[4_137_074_001, 4_137_074_002],
        &["ada-opens-sealed", "bob-seals-to-ada"],
        false,
    );
    let from_bob = talk.address(&talk.played[0].person);
    let (at, ada, bob) = (talk.at(), talk.ada.device, talk.played[0].device());
    for (counter, text) in (1..).zip(OVER_MLS) {
        bob_sends(&mut talk, counter, text);
    }
    let over_mls = received(&talk.ada.machine).shown();
    for text in OVER_MLS {
        assert_shows_once(&over_mls, text, (at.1, &from_bob, BOB_HOST));
    }
    let key = cert_of(talk.cloud(), &talk.ada).body().hpke;
    let seal = |counter: u64, text: &str| {
        let line = Line::new(&from_bob, counter, text);
        sealed(&talk.played[0].person, at, (ada, &key), &line)
    };
    let held = seal(1, HELD);
    let second = sends(&talk, &seal(2, SECOND));

    let shown = received(&talk.ada.machine).shown();

    assert_shows_once(&shown, SECOND, (at.1, &from_bob, BOB_HOST));
    assert!(
        marked(&shown, second),
        "the sealed message not marked: {shown}"
    );
    assert_reported_once(&shown, at.1, bob);
    sends(&talk, &seal(2, REPEATED));
    sends(&talk, &held);
    talk.front.inject(ada, vec![held.clone()]);

    let next = received(&talk.ada.machine).shown();

    assert_eq!(talk.front.injected(), 1, "not repeated: {next}");
    assert!(
        !next.contains(REPEATED),
        "the repeated counter shown: {next}"
    );
    assert_shows_once(&next, HELD, (at.1, &from_bob, BOB_HOST));
    assert_none_reported(&next);
}
