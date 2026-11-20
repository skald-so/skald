//! Handshake messages, a channel's Commits and Welcomes, are never refused for the rate and do not
//! count towards it.

use crate::rate::{RATE, RATE_LIMITED};
use crate::scene::{Scene, empty};

/// With the setting at 1, alice forms the channel's group: her Commit and its Welcome are stored.
/// Her first application message is stored too, and her second is `rate-limited`. Past the rate,
/// her Commit that adds dave and its Welcome are stored, and so is her empty Commit after it; her
/// next application message is still `rate-limited`.
#[test]
fn commits_and_welcomes_are_never_refused_for_the_rate() {
    let (mut s, _) = Scene::formed_with(3_216_060_301, &[RATE, "1"]);
    let f = &s.fixture;
    let first = (s.alice.mls.encrypt(f.general, b"the first")).expect("a message");
    s.alice.stores(f, &first, &[]);
    let second = (s.alice.mls.encrypt(f.general, b"the second")).expect("a message");
    s.alice
        .assert_refused(f, &second, &[], RATE_LIMITED, "the second message");

    s.add_dave();
    let f = &s.fixture;
    let commit = s.alice.commits(f, empty);
    for peer in [&mut s.bob, &mut s.carol, &mut s.dave] {
        peer.process(f, &commit);
    }
    let third = (s.alice.mls.encrypt(f.general, b"the third")).expect("a message");
    s.alice
        .assert_refused(f, &third, &[], RATE_LIMITED, "the third message");
}
