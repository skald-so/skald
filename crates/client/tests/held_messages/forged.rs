//! The sending device takes `held` only from the message's `to` agent.

use skald_proto::{Body, NonEmpty, Receipt, ReceiptStatus, Unknown};

use crate::fixtures::TO;
use crate::held::{HELD, shows_not, shows_once};
use crate::receipts::Roles;
use crate::senders::{BOB_AGENT, LAPTOP_AGENT};
use crate::talk::Line;
use crate::world::{ACME, HOST};

/// Another agent of Bob's, which the message does not go to.
const BOB_OTHER: &str = "codex-w8k2rd";

/// A's agent sends a message `--to` Bob's agent. Bob's device sends a `held` receipt of it from
/// another agent of his: A's person's `skald recv` shows no `held`. Bob's device then sends one from
/// the agent the message went to: A's runs show `held` of it once.
#[test]
fn held_counts_only_from_the_message_s_to_agent() {
    let mut roles = Roles::new(
        [4_739_011_001, 4_739_011_002],
        ["ada-agent-gets-kept-notes", "bob-forges-kept-notes"],
    );
    let bob = roles.senders.bob();
    let to_bob = format!("{bob}/{BOB_AGENT}");
    let msg_id = roles.a_sends(
        &[TO, &to_bob],
        "hold forgery test: bob's agent, ship the patch",
    );
    let a_agent = roles.full(LAPTOP_AGENT);

    for (counter, agent) in [(1, BOB_OTHER), (2, BOB_AGENT)] {
        let held = Body::Receipt(Receipt {
            messages: NonEmpty::new(msg_id),
            status: ReceiptStatus::Held,
            unknown: Unknown::default(),
            reason: None,
        });
        let from = format!("{ACME}/{bob}/{agent}@{HOST}");
        let mut line = Line::new(&from, counter, "");
        line.to = Some(a_agent.clone());
        line.body = Some(held);
        roles.senders.agents.bob_sends(&line);
        if agent == BOB_OTHER {
            shows_not(roles.a(), msg_id, HELD, "from another agent");
        }
    }

    shows_once(roles.a(), msg_id, HELD);
}
