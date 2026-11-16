//! A handover after missing messages is marked as following them.

use crate::gaps::{marked, second_dropped};
use crate::kit::received;
use crate::{Agents, CLAUDE, assert_handed_once, assert_not_handed};

/// The texts Bob's device sends; none is anywhere else.
const SAYS: [&str; 3] = [
    "nightly build started",
    "nightly build failed on the arm runner",
    "nightly build green after a retry",
];

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends three messages to the
/// channel; the second never reaches the server. The person's `skald recv` shows the first and the
/// third; claude's next run then gets them too, the third marked as following missing ones and the
/// first not.
#[test]
fn message_after_a_missing_one_reaches_the_agent_marked() {
    let mut agents = Agents::new(
        [4_147_073_001, 4_147_073_002],
        ["ada-agent-sees-gap", "bob-loses-one-to-agent"],
    );
    agents.run_as(CLAUDE);
    let (first, third) = second_dropped(&mut agents.talk, SAYS);
    let person = received(&agents.talk.ada.machine).shown();
    assert!(
        marked(&person, third),
        "not marked for the person: {person}"
    );

    let claude = agents.run_as(CLAUDE).shown();

    assert_handed_once(&claude, SAYS[0], first);
    assert_handed_once(&claude, SAYS[2], third);
    assert_not_handed(&claude, SAYS[1]);
    assert!(
        !marked(&claude, first),
        "the first message marked: {claude}"
    );
    assert!(
        marked(&claude, third),
        "the third message not marked: {claude}"
    );
}
