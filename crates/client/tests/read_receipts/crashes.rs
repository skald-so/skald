//! The receipt is kept with the mark: a CLI killed after the mark and before the receipt is sent
//! neither loses it nor sends it twice.

use crate::fixtures::kill_once;
use crate::receipts::{Roles, shows_no_read, shows_read_once};
use crate::talk::messages_sent;

/// What A's agent sends; it is nowhere else.
const TEXT: &str = "receipt crash test: claude, rotate the signing key";

/// A's agent sends a message `--to` claude. With the front swallowing every `send`, claude's
/// `skald recv` hands the message over and is killed once it has sent a message in the channel,
/// which the server never got; A shows no `read`. With the sends forwarded, claude's next run
/// shows the message no more, and A's runs show `read` once. C reads one receipt of it.
#[test]
fn receipt_of_a_cli_killed_before_sending_it_is_sent_by_the_next_run_once() {
    let mut roles = Roles::new(
        [4_155_006_001, 4_155_006_002],
        ["ada-agent-killed-midway", "bob-counts-receipts"],
    );
    let msg_id = roles.a_sends_to_claude(TEXT);
    let channel = roles.channel();
    let front = &roles.senders.agents.talk.front;
    let from = front.sent().len();
    front.swallow(true);

    kill_once(roles.claude_starts(&[]), "swallowed receipt", || {
        !messages_sent(front, channel, from).is_empty()
    });

    front.swallow(false);
    shows_no_read(roles.a(), msg_id, "with the receipt swallowed");

    let next = roles.claude_reads(false);

    assert!(!next.shown().contains(TEXT), "handed over again: {next:?}");
    shows_read_once(roles.a(), msg_id);
    let receipts = roles.c_receipts();
    let of_it = receipts.iter().filter(|got| got.lists(msg_id)).count();
    assert_eq!(of_it, 1, "{receipts:#?}");
}
