//! What goes to no agent: content to a person, and a receipt.

use skald_proto::{Body, MessageId, NonEmpty, Receipt, ReceiptStatus, Unknown};

use crate::kit::received;
use crate::lines::{AgentLine, agent_line, lines_referring_to};
use crate::talk::{BOB_HOST, Line, assert_shows_once};
use crate::world::{ACME, HOST};
use crate::{Agents, CLAUDE, assert_handed_line, assert_not_handed};

/// The texts Bob's device sends; none is anywhere else.
const TO_ALL: &str = "the release branch is cut";
const TO_ADA: &str = "ada, can you sign the release notes yourself";

/// Bob's agent, as the `from` of its receipt.
const BOB_AGENT: &str = "claude-b4n6ze";

/// Ada's device has run as claude in `~/work/acme`. Bob's device sends to the channel a message
/// without `to`, one to Ada as a person, and, as Bob's agent, a `read` receipt to claude. claude's
/// next run shows only the first: no message line holds a reference of the other two. The
/// person's run then shows the message to Ada once.
#[test]
fn message_to_a_person_and_a_receipt_go_to_no_agent() {
    let mut agents = Agents::new(
        [4_147_072_001, 4_147_072_002],
        ["ada-gets-person-mail", "bob-mails-ada-herself"],
    );
    agents.run_as(CLAUDE);
    let bob = agents.bob();
    let to_all = agents.bob_sends(&Line::new(&bob, 1, TO_ALL));
    let mut line = Line::new(&bob, 2, TO_ADA);
    line.to = Some(agents.talk.address(&agents.talk.ada));
    let to_ada = agents.bob_sends(&line);
    let about = MessageId::generate();
    let read = Body::Receipt(Receipt {
        messages: NonEmpty::new(about),
        status: ReceiptStatus::Read,
        unknown: Unknown::default(),
        reason: None,
    });
    let handle = &agents.talk.played[0].person.handle;
    let mut receipt = Line::new(&format!("{ACME}/{handle}/{BOB_AGENT}@{HOST}"), 3, "");
    (receipt.body, receipt.to) = (Some(read), Some(agents.address(CLAUDE)));
    let receipt = agents.bob_sends(&receipt);

    let claude = agents.run_as(CLAUDE).shown();

    assert_handed_line(&claude, TO_ALL, to_all);
    assert_not_handed(&claude, TO_ADA);
    for id in [to_ada, receipt] {
        let handed: Vec<AgentLine> = (lines_referring_to(&claude, id).iter())
            .filter_map(|line| agent_line(line))
            .collect();
        assert!(handed.is_empty(), "{id} handed over: {handed:?}");
    }

    let person = received(&agents.talk.ada.machine).shown();

    let channel = agents.talk.channel;
    assert_shows_once(&person, TO_ADA, (channel, &bob, BOB_HOST));
}
