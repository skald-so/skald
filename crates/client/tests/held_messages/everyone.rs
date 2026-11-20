//! A message to every agent, held for an agent in `hold`, gets no receipt and, once accepted,
//! reaches that agent.

use skald_proto::{MessageId, ReceiptStatus};

use crate::agents::{CLAUDE, CODEX};
use crate::fixtures::ALL;
use crate::held::{Decision, HELD, c_reads, decide, holding, says_waiting, status_lines};
use crate::inbound::REFUSED;
use crate::receipts::{READ, Roles, assert_shows_once, shows_read};
use crate::senders::LAPTOP_AGENT;

/// What A's agent sends; none is anywhere else.
const TO_ALL: &str = "hold everyone test: everyone, the freeze starts at noon";
const TO_CLAUDE: &str = "hold everyone test: claude, cut the release branch";
const TEXTS: [&str; 2] = [TO_ALL, TO_CLAUDE];

/// Fails if C read a receipt, whatever its status, that lists `msg_id`.
#[track_caller]
fn no_receipt_of(roles: &mut Roles, msg_id: MessageId) {
    let all = roles.c_receipts();
    let of_it: Vec<_> = all.iter().filter(|got| got.lists(msg_id)).collect();
    assert!(
        of_it.is_empty(),
        "a receipt of a message to every agent: {of_it:#?}"
    );
}

/// B's person sets claude to `hold`; B's device has run as codex too, in `accept`. A's agent sends
/// a message `--all`, then one `--to` claude. codex's `skald recv` gets the first, claude's gets
/// neither, and B's person's `skald recv` says two messages wait. C reads B's `held` receipt of the
/// second and none of the first. The person's `skald held` accepts both: claude's next
/// `skald recv` gets both, C reads B's `read` receipt of the second and still none of the first,
/// and A's runs show `read` of the second and nothing of the first.
#[test]
fn held_message_to_every_agent_gets_no_receipt_and_once_accepted_reaches_the_agent() {
    let mut roles = holding(
        [4_739_007_001, 4_739_007_002],
        ["ada-agents-all-kept", "bob-counts-kept-receipts"],
    );
    let agents = &roles.senders.agents;
    agents.run_as(CODEX);
    let to_all = roles.a_sends(&[ALL], TO_ALL);
    let to_claude = roles.a_sends_to_claude(TO_CLAUDE);

    assert_shows_once(&agents.run_as(CODEX).shown(), TO_ALL);
    let before = roles.claude_reads(false).shown();
    let handed: Vec<&str> = (TEXTS.into_iter())
        .filter(|text| before.contains(text))
        .collect();
    assert!(handed.is_empty(), "{CLAUDE} got {handed:?}: {before}");
    says_waiting(roles.b(), 2);
    let to = roles.full(LAPTOP_AGENT);
    c_reads(&mut roles, to_claude, ReceiptStatus::Held, &to);
    no_receipt_of(&mut roles, to_all);

    let folder = roles.senders.agents.folder.clone();
    let both = [(TO_ALL, Decision::Accept), (TO_CLAUDE, Decision::Accept)];
    decide(roles.b(), &folder, &both);

    let after = roles.claude_reads(false).shown();
    for text in TEXTS {
        assert_shows_once(&after, text);
    }
    c_reads(&mut roles, to_claude, ReceiptStatus::Read, &to);
    no_receipt_of(&mut roles, to_all);
    let shown = shows_read(roles.a(), to_claude);
    for status in [HELD, READ, REFUSED] {
        let lines = status_lines(&shown, to_all, status);
        assert!(lines.is_empty(), "{CLAUDE}: {lines:?} in {shown}");
    }
}
