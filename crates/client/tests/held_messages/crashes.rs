//! A decision and its receipt are kept together: a CLI killed after a decision and before its
//! receipt went neither loses nor repeats them, and an accepted message is handed over once.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::ReceiptStatus;

use crate::agents::assert_not_handed;
use crate::cloud::WAIT;
use crate::fixtures::killed_once;
use crate::held::{
    Decision, answer, c_reads, decide, deciding, holding, nothing_to_decide, receipts_with,
    says_none_waiting, says_waiting, shows_once,
};
use crate::inbound::{REFUSED, shows_not};
use crate::receipts::{READ, assert_shows_once, shows_read_once};
use crate::senders::LAPTOP_AGENT;
use crate::talk::messages_sent;

/// What the person types when refusing.
const REASON: &str = "wait for the change window";

/// B's person sets claude to `hold`, and A's agent sends a message `--to` claude, which waits; C
/// reads B's `held` receipt of it. With the front swallowing every `send`, the person's
/// `skald held` refuses it with a reason and is killed once it has sent a message in the channel,
/// which the server never got; A shows no `refused`. With the sends forwarded, B's person's next
/// `skald recv` says none waits; A's runs show `refused` once with the reason, and C reads one
/// `refused` receipt of the message. The person's `skald held` has nothing to decide, and claude's
/// `skald recv` gets nothing.
#[test]
fn refusal_of_a_cli_killed_before_its_receipt_went_is_sent_by_the_next_run_once() {
    let mut roles = holding(
        [4_739_008_001, 4_739_008_002],
        ["ada-agent-turned-down-midcrash", "bob-counts-one-refusal"],
    );
    let text = "hold crash test: claude, truncate the events table";
    let msg_id = roles.a_sends_to_claude(text);
    says_waiting(roles.b(), 1);
    let to = roles.full(LAPTOP_AGENT);
    c_reads(&mut roles, msg_id, ReceiptStatus::Held, &to);
    let (channel, folder) = (roles.channel(), roles.senders.agents.folder.clone());
    let front = &roles.senders.agents.talk.front;
    let mark = front.sent().len();
    front.swallow(true);

    let mut terminal = deciding(roles.b(), &folder);
    terminal.wait_to_show(text);
    answer(&terminal, Decision::Refuse(REASON));
    let deadline = Instant::now() + WAIT;
    while messages_sent(front, channel, mark).is_empty() {
        assert!(
            Instant::now() < deadline,
            "no refusal sent: {}",
            terminal.shown()
        );
        thread::sleep(Duration::from_millis(50));
    }
    terminal.kill();
    front.swallow(false);
    shows_not(roles.a(), msg_id, REFUSED, "with the receipt swallowed");

    says_none_waiting(roles.b(), "after the killed decision");

    let line = shows_once(roles.a(), msg_id, REFUSED);
    assert!(line.contains(REASON), "no reason {REASON:?}: {line}");
    let refusals = receipts_with(&mut roles, msg_id, ReceiptStatus::Refused);
    assert_eq!(refusals.len(), 1, "{refusals:#?}");
    nothing_to_decide(roles.b(), &folder, &[text]);
    assert_not_handed(&roles.claude_reads(false).shown(), text);
}

/// B's person sets claude to `hold`, A's agent sends a message `--to` claude, and the person's
/// `skald held` accepts it. With the front swallowing every `send`, claude's `skald recv` is killed
/// once it has sent a message in the channel; A shows no `read`. With the sends forwarded,
/// claude's runs together show the message once; A's runs show `read` once, C reads one `read`
/// receipt of it, and the person's `skald held` has nothing to decide.
#[test]
fn accepted_message_of_a_cli_killed_before_its_receipt_went_is_handed_over_once() {
    let mut roles = holding(
        [4_739_009_001, 4_739_009_002],
        ["ada-agent-accepted-midcrash", "bob-counts-one-handover"],
    );
    let text = "hold crash test: claude, restart the ingest workers";
    let msg_id = roles.a_sends_to_claude(text);
    says_waiting(roles.b(), 1);
    let folder = roles.senders.agents.folder.clone();
    decide(roles.b(), &folder, &[(text, Decision::Accept)]);
    let channel = roles.channel();
    let front = &roles.senders.agents.talk.front;
    let mark = front.sent().len();
    front.swallow(true);

    let first = killed_once(roles.claude_starts(&[]), "swallowed read", || {
        !messages_sent(front, channel, mark).is_empty()
    });
    front.swallow(false);
    shows_not(roles.a(), msg_id, READ, "with the receipt swallowed");

    let next = roles.claude_reads(false);

    assert_shows_once(&format!("{}\n{}", first.shown(), next.shown()), text);
    shows_read_once(roles.a(), msg_id);
    let reads = receipts_with(&mut roles, msg_id, ReceiptStatus::Read);
    assert_eq!(reads.len(), 1, "{reads:#?}");
    nothing_to_decide(roles.b(), &folder, &[text]);
}
