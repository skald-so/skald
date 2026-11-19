//! A client gone before `inbox`'s result is written: the next session's `inbox` returns the
//! message; once written, never again (`held.rs`).

use crate::agents::Agents;
use crate::held::{HEAD, long_text};
use crate::lines::assert_handed_line;
use crate::talk::Line;
use crate::{CLIENT, assert_nothing_waiting, session_in};

/// A session of `claude-code` has started in `~/work/acme`. Bob's device sends a message with a
/// long text to the channel. The session's client is gone in the middle of the result of its
/// `inbox`, with most of the text unread. The next session of `claude-code` there gets the message
/// from `inbox`, once, by its reference; its next `inbox`, and that of the session after it, give
/// one short line.
#[test]
fn message_of_an_inbox_whose_client_is_gone_goes_to_the_next_session_once() {
    let mut agents = Agents::new(
        [4_627_011_001, 4_627_011_002],
        ["ada-session-crashes", "bob-sends-the-long-export"],
    );
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);
    let gone = session_in(machine, folder, &[], CLIENT);
    let bob = agents.bob();
    let text = long_text();
    let id = agents.bob_sends(&Line::new(&bob, 1, &text));
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);

    let read = gone.gone_in_the_middle_of_inbox(HEAD);

    assert!(!read.contains(&text), "the text written whole");
    let mut next = session_in(machine, folder, &[], CLIENT);
    let shown = next.inbox();
    assert_handed_line(&shown, &text, id);
    assert_nothing_waiting(&next.inbox());
    drop(next);
    let mut after = session_in(machine, folder, &[], CLIENT);
    assert_nothing_waiting(&after.inbox());
}
