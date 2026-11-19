//! While B's device's Commit in the channel waits for the server, the receipt waits; the handover
//! does not.

use crate::front::Answers;
use crate::receipts::{Roles, assert_shows_once, shows_read_once};
use crate::talk::{commit_sent, messages_sent};

/// What A's agent sends; it is nowhere else.
const TEXT: &str = "receipt commit test: claude, drain the old queue";

/// A's agent sends a message `--to` claude; then a revision that B's device signs takes Bob out of
/// the channel. With every `stored` lost, claude's `skald recv` shows the message and commits the
/// Remove of Bob's device, whose `stored` it never gets, and sends no message in the channel. With
/// the answers forwarded, claude's next run shows the message no more, and A's runs show `read`
/// once.
#[test]
fn receipt_waits_for_the_stored_of_a_remove_and_the_handover_does_not() {
    let roles = Roles::new(
        [4_155_007_001, 4_155_007_002],
        ["ada-agent-behind-a-remove", "bob-taken-out-later"],
    );
    let msg_id = roles.a_sends_to_claude(TEXT);
    roles.c_leaves();
    let (talk, channel) = (&roles.senders.agents.talk, roles.channel());
    talk.front.answer(Answers::Lose);
    let from = talk.front.sent().len();

    let held = roles.claude_starts_retrying().finish();

    assert_shows_once(&held.stdout, TEXT);
    assert!(talk.front.lost() > 0, "no stored was lost: {held:?}");
    assert!(
        commit_sent(&talk.front, channel, from),
        "no Remove: {held:?}"
    );
    let sent = messages_sent(&talk.front, channel, from);
    assert!(sent.is_empty(), "a message before the Remove: {sent:?}");
    talk.front.answer(Answers::Forward);

    let next = roles.claude_reads(false);

    assert!(!next.shown().contains(TEXT), "handed over again: {next:?}");
    shows_read_once(roles.a(), msg_id);
}
