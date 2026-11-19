//! A device other than the sender of a receipt's messages drops the receipt without a word, and
//! counts its `counter` in gap detection.

use skald_proto::{Body, MessageId, NonEmpty, Receipt, ReceiptStatus, Unknown};

use crate::gaps::reports_of;
use crate::kit::{received, warnings_naming};
use crate::receipts::read_lines;
use crate::talk::{Line, Talk, encrypted, stored};
use crate::world::{ACME, HOST};

/// Bob's and Carol's devices among the talk's played ones.
const BOB: usize = 0;
const CAROL: usize = 1;

/// Bob's agent and Carol's.
const BOB_AGENT: &str = "claude-p5m9hs";
const CAROL_AGENT: &str = "codex-t3k8vb";

/// What Carol's and Bob's agents say; none is anywhere else.
const CAROL_ASKS: &str = "receipt bystander test: bob's agent, check the invoice totals";
const BOB_FIRST: &str = "receipt bystander test: bob's agent starts on the invoices";
const BOB_NEXT: &str = "receipt bystander test: bob's agent finished the invoices";

/// The address of the agent `agent` of the talk's played person `who` in `acme`.
fn agent_of(talk: &Talk, who: usize, agent: &str) -> String {
    let handle = &talk.played[who].person.handle;
    format!("{ACME}/{handle}/{agent}@{HOST}")
}

/// The talk's played device `who` says `line` in the channel; fails unless the server stores it.
/// Gives its `msg_id`.
#[track_caller]
fn says(talk: &mut Talk, who: usize, line: &Line) -> MessageId {
    let at = talk.at();
    let envelope = encrypted(&mut talk.played[who], at, at.1, line);
    stored(&talk.world.cloud, &talk.played[who], &envelope);
    envelope.envelope().header.msg_id
}

/// Carol's agent asks Bob's agent in the channel. Bob's agent says a line to everyone, then Bob's
/// device sends a `read` receipt of Carol's message from his agent to hers. Ada's `skald recv`
/// shows both messages and neither the receipt nor `read` of Carol's message, and warns of
/// nothing from Bob's device. Bob's agent then says another line; Ada's next `skald recv` shows it
/// and reports no message of Bob's device missing.
#[test]
fn device_other_than_the_sender_drops_a_receipt_without_a_word_and_counts_it() {
    let mut talk = Talk::new(
        &[4_155_009_001, 4_155_009_002, 4_155_009_003],
        &["ada-only-watches", "bob-sends-a-receipt", "carol-asks-bob"],
        false,
    );
    let (bob_agent, carol_agent) = (
        agent_of(&talk, BOB, BOB_AGENT),
        agent_of(&talk, CAROL, CAROL_AGENT),
    );
    let mut asks = Line::new(&carol_agent, 1, CAROL_ASKS);
    asks.to = Some(bob_agent.clone());
    let asked = says(&mut talk, CAROL, &asks);
    says(&mut talk, BOB, &Line::new(&bob_agent, 1, BOB_FIRST));
    let mut receipt = Line::new(&bob_agent, 2, "");
    receipt.to = Some(carol_agent);
    receipt.body = Some(Body::Receipt(Receipt {
        status: ReceiptStatus::Read,
        messages: NonEmpty::new(asked),
        reason: None,
        unknown: Unknown::default(),
    }));
    let receipt = says(&mut talk, BOB, &receipt);
    let bob = talk.played[BOB].device();

    let first = received(&talk.ada.machine);

    let shown = first.shown();
    for text in [CAROL_ASKS, BOB_FIRST] {
        assert!(shown.contains(text), "{text:?} not shown: {shown}");
    }
    assert!(
        !shown.contains(&receipt.to_string()),
        "the receipt: {shown}"
    );
    assert_eq!(read_lines(&shown, asked), 0, "{shown}");
    assert_eq!(warnings_naming(&first, bob), Vec::<String>::new());

    says(&mut talk, BOB, &Line::new(&bob_agent, 3, BOB_NEXT));

    let next = received(&talk.ada.machine);

    let shown = next.shown();
    assert!(shown.contains(BOB_NEXT), "not shown: {shown}");
    assert_eq!(reports_of(&shown, talk.channel, bob), Vec::<String>::new());
    assert_eq!(warnings_naming(&next, bob), Vec::<String>::new());
}
