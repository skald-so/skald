//! A message to every agent reaches no agent in `refuse` and gets no receipt; another agent of the
//! device in `accept` gets it.

use crate::agents::{CODEX, assert_not_handed};
use crate::fixtures::ALL;
use crate::inbound::{REFUSE, REFUSED, claude_set_to, shows_refused_once, status_lines};
use crate::receipts::{Roles, assert_shows_once};

/// What A's agent sends; none is anywhere else.
const TO_ALL: &str = "inbound everyone test: everyone, the queue is draining";
const TO_CLAUDE: &str = "inbound everyone test: claude, pause the workers";

/// B's device has run as codex too; B's person sets claude to `refuse`. A's agent sends a message
/// `--all`, then one `--to` claude. claude's `skald recv` gets neither; codex's gets the first. C
/// reads B's receipt of the second, and no receipt of the first. A's runs show `refused` of the
/// second once, and none of the first.
#[test]
fn message_to_everyone_reaches_no_agent_in_refuse_and_gets_no_receipt() {
    let mut roles = Roles::new(
        [4_731_008_001, 4_731_008_002],
        ["ada-agents-told-at-once", "bob-counts-no-receipt"],
    );
    roles.senders.agents.run_as(CODEX);
    claude_set_to(&roles, REFUSE);
    let to_all = roles.a_sends(&[ALL], TO_ALL);
    let to_claude = roles.a_sends_to_claude(TO_CLAUDE);

    let claude = roles.claude_reads(false).shown();
    let codex = roles.senders.agents.run_as(CODEX).shown();

    for text in [TO_ALL, TO_CLAUDE] {
        assert_not_handed(&claude, text);
    }
    assert_shows_once(&codex, TO_ALL);
    let receipts = roles.c_gets_receipt_of(to_claude);
    let of_all: Vec<_> = receipts.iter().filter(|got| got.lists(to_all)).collect();
    assert!(
        of_all.is_empty(),
        "a receipt of a message to everyone: {of_all:#?}"
    );
    let shown = shows_refused_once(roles.a(), to_claude);
    assert!(status_lines(&shown, to_all, REFUSED).is_empty(), "{shown}");
    assert_not_handed(&roles.claude_reads(false).shown(), TO_ALL);
}
