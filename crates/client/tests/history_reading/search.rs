//! Searching the history runs on the client: it shows only the messages whose text holds the
//! searched text, and no request carries that text.

use crate::keys::holds;
use crate::reading::{Duo, assert_history, history, sends};

/// The text searched; it is in no other request, login or name.
const SEARCHED: &str = "quokka";

/// Ada and Bob say four lines, two of them holding [`SEARCHED`]. Ada's `skald history <channel>
/// quokka` shows those two, oldest first, and neither of the others; no byte the machines sent
/// through the recording relay, in any encoding, holds the searched text.
#[test]
fn search_shows_only_the_matching_messages_and_no_request_carries_its_text() {
    let mut duo = Duo::new(
        &[4_166_012_001, 4_166_012_002],
        &["ada-searches-history", "bob-fills-history"],
    );
    let channel = (duo.talk.space.name.clone(), duo.at.1);
    let in_channel = (channel.0.as_str(), channel.1);
    let first = sends(&duo.talk.ada, in_channel, "the quokka build is green");
    let (other, ..) = duo.bob_says(&duo.bob_s(1, "lunch is at noon"));
    let (second, ..) = duo.bob_says(&duo.bob_s(2, "a quokka ate the cable"));
    let last = sends(&duo.talk.ada, in_channel, "release notes are out");

    let found = history(&duo.talk.ada.machine, channel.1, Some(SEARCHED));

    assert!(found.success, "{found:?}");
    let absent = [other.text.as_str(), last.text.as_str()];
    assert_history(&found, channel.1, &[first, second], &absent);
    let sent = duo.talk.world.cloud.relay.sent_bytes();
    assert!(
        !holds(&sent, SEARCHED.as_bytes()),
        "a request carries the searched text"
    );
}
