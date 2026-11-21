//! A sender on the allowlist of an agent in `hold` is handed over at once, with no `held`.

use skald_proto::ReceiptStatus;

use crate::held::{HELD, c_reads, holding, receipts_with, says_none_waiting};
use crate::inbound::{agent_status_lines, claude_allows, status_lines};
use crate::receipts::{assert_shows_once, shows_read};
use crate::senders::LAPTOP_AGENT;

/// B's person sets claude to `hold` and puts A's agent on its allowlist by its short address. A's
/// agent sends a message `--to` claude: claude's next `skald recv` gets it, and no message waits
/// for a decision. A's runs show `read` of it and no `held`; A's agent says no `held`; C
/// reads B's `read` receipt of it and no `held` one.
#[test]
fn sender_on_the_allowlist_of_an_agent_in_hold_is_handed_over_at_once_without_held() {
    let mut roles = holding(
        [4_739_005_001, 4_739_005_002],
        ["ada-agent-let-straight-in", "bob-sees-no-hold"],
    );
    claude_allows(&roles, &format!("{}/{LAPTOP_AGENT}", roles.senders.ada()));
    let text = "hold test: claude, bump the patch version";
    let msg_id = roles.a_sends_to_claude(text);

    assert_shows_once(&roles.claude_reads(false).shown(), text);
    says_none_waiting(roles.b(), "a sender on the allowlist");
    let to = roles.full(LAPTOP_AGENT);
    c_reads(&mut roles, msg_id, ReceiptStatus::Read, &to);
    let shown = shows_read(roles.a(), msg_id);
    let lines = status_lines(&shown, msg_id, HELD);
    assert!(lines.is_empty(), "{lines:?} in {shown}");
    let agent = roles.a_agent_reads(&[]).shown();
    let said = agent_status_lines(&agent, msg_id, HELD);
    assert!(said.is_empty(), "{said:?} in {agent}");
    let held = receipts_with(&mut roles, msg_id, ReceiptStatus::Held);
    assert!(held.is_empty(), "{held:#?}");
}
