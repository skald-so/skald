//! `read` only for messages to an agent: a message sent with `--all` gets none. One receipt
//! reports on the messages of one sending device and one `from` that a handover marks.

use std::collections::BTreeSet;

use skald_proto::{MessageId, ReceiptStatus};

use crate::agents::CLAUDE;
use crate::fixtures::ALL;
use crate::receipts::{Roles, read_lines, shows_read};
use crate::senders::LAPTOP_AGENT;

/// What A's agent sends; none is anywhere else.
const TO_ALL: &str = "receipt addressee test: everyone, the cache is cold";
const TO_CLAUDE: &str = "receipt addressee test: claude, warm the cache";
const AGAIN: &str = "receipt addressee test: claude, then purge the CDN";

/// A's agent sends a message `--all`, then two `--to` claude, and claude's one `skald recv` gets all
/// three. C reads one receipt of B's device in the channel: `read` of the two messages to claude
/// and of no other, from claude's address to A's agent's, with `hops` 0, sent with `report` false.
/// A shows `read` of both messages to claude and none of the one to everyone.
#[test]
fn message_to_everyone_gets_no_read_and_one_receipt_reports_both_to_the_agent() {
    let mut roles = Roles::new(
        [4_155_005_001, 4_155_005_002],
        ["ada-agent-tells-everyone", "bob-reads-the-receipt"],
    );
    let to_all = roles.a_sends(&[ALL], TO_ALL);
    let to_claude = roles.a_sends_to_claude(TO_CLAUDE);
    let again = roles.a_sends_to_claude(AGAIN);

    let got = roles.claude_reads(false).stdout;

    for text in [TO_ALL, TO_CLAUDE, AGAIN] {
        assert!(got.contains(text), "claude did not get {text:?}: {got}");
    }
    let receipts = roles.c_gets_receipt_of(to_claude);
    let [receipt] = receipts.as_slice() else {
        panic!("not one receipt: {receipts:#?}");
    };
    let listed: BTreeSet<MessageId> = receipt.receipt.messages.iter().copied().collect();
    assert_eq!(listed, BTreeSet::from([to_claude, again]), "{receipt:#?}");
    assert!(!receipt.lists(to_all), "{receipt:#?}");
    assert_eq!(receipt.receipt.status, ReceiptStatus::Read, "{receipt:#?}");
    assert_eq!(receipt.sender, roles.b_device(), "{receipt:#?}");
    let content = &receipt.content;
    assert_eq!(content.from.to_string(), roles.full(CLAUDE), "{receipt:#?}");
    assert_eq!(
        content.to.as_ref().map(ToString::to_string),
        Some(roles.full(LAPTOP_AGENT)),
        "{receipt:#?}"
    );
    assert_eq!(content.hops, 0, "{receipt:#?}");
    let front = &roles.senders.agents.talk.front;
    let sent: Vec<bool> = (front.sent().into_iter())
        .filter(|sent| sent.envelope.envelope().header.msg_id == receipt.msg_id)
        .map(|sent| sent.report)
        .collect();
    assert!(
        !sent.is_empty() && sent.iter().all(|report| !report),
        "not sent with report false: {sent:?}"
    );

    let shown = shows_read(roles.a(), to_claude);

    let shown = shown + &crate::kit::received(roles.a()).shown();
    assert_eq!(read_lines(&shown, again), 1, "{shown}");
    assert_eq!(read_lines(&shown, to_all), 0, "{shown}");
}
