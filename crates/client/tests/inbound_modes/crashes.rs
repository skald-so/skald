//! `refused` is kept with the decision: a CLI killed after the decision and before the receipt is
//! sent neither loses it nor sends it twice.

use crate::agents::assert_not_handed;
use crate::fixtures::kill_once;
use crate::inbound::{REFUSE, REFUSED, claude_set_to, shows_not, shows_refused_once};
use crate::receipts::Roles;
use crate::talk::messages_sent;

/// What A's agent sends; it is nowhere else.
const TEXT: &str = "inbound crash test: claude, purge the audit log";

/// B's person sets claude to `refuse`, and A's agent sends a message `--to` claude. With the front
/// swallowing every `send`, claude's `skald recv` is killed once it has sent a message in the
/// channel, which the server never got; A shows no `refused`. With the sends forwarded, claude's
/// next run gets nothing, A's runs show `refused` once, and C reads one receipt of the message.
#[test]
fn refusal_of_a_cli_killed_before_its_receipt_went_is_sent_by_the_next_run_once() {
    let mut roles = Roles::new(
        [4_731_010_001, 4_731_010_002],
        ["ada-agent-killed-midstep", "bob-counts-the-refusals"],
    );
    claude_set_to(&roles, REFUSE);
    let msg_id = roles.a_sends_to_claude(TEXT);
    let channel = roles.channel();
    let front = &roles.senders.agents.talk.front;
    let mark = front.sent().len();
    front.swallow(true);

    kill_once(roles.claude_starts(&[]), "swallowed refusal", || {
        !messages_sent(front, channel, mark).is_empty()
    });

    front.swallow(false);
    shows_not(roles.a(), msg_id, REFUSED, "with the receipt swallowed");

    let next = roles.claude_reads(false);

    assert_not_handed(&next.shown(), TEXT);
    shows_refused_once(roles.a(), msg_id);
    let receipts = roles.c_receipts();
    let of_it: Vec<_> = receipts.iter().filter(|got| got.lists(msg_id)).collect();
    assert_eq!(of_it.len(), 1, "{receipts:#?}");
}
