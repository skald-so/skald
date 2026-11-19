//! The sending device takes `refused` only from the message's `to` agent, for its message in that
//! channel; it ignores any other.

use skald_proto::{Body, ChannelId, MessageId, NonEmpty, Receipt, ReceiptStatus, Unknown};

use crate::{
    agents::{Agents, CLAUDE},
    fixtures::{CHANNEL, SEND, TO},
    inbound::{REFUSED, refused_lines, shows_refused_once},
    kit::received,
    talk::{Line, one_message_sent},
    world::{ACME, AGENT, HOST},
};

/// Bob's agent claude's messages go to, and another of his agents.
const BOB_AGENT: &str = "claude-p2v7ke";
const BOB_OTHER: &str = "codex-t9f3wm";

/// What claude sends; none is anywhere else.
const IN_CHANNEL: &str = "inbound forgery test: bob's agent, rotate the tokens";
const IN_DEFAULT: &str = "inbound forgery test: bob's agent, rotate the certificates";

/// The reason in Bob's receipts.
const REASON: &str = "this agent takes messages from its allowlist only";

/// Runs `skald --agent claude send --channel <channel> --to <bob>/<BOB_AGENT> <text>` in
/// `~/work/acme`; fails unless it succeeds. Gives the `msg_id` of the message it sent.
#[track_caller]
fn claude_sends(agents: &Agents, channel: ChannelId, text: &str) -> MessageId {
    let front = &agents.talk.front;
    let mark = front.sent().len();
    let bob = &agents.talk.played[0].person.handle;
    let (to, id) = (format!("{bob}/{BOB_AGENT}"), channel.to_string());
    let args = [AGENT, CLAUDE, SEND, CHANNEL, &id, TO, &to, text];
    let run = agents.talk.ada.machine.start_in(&agents.folder, &args);
    let sent = run.finish();
    assert!(sent.success, "claude: {sent:?}");
    one_message_sent(front, channel, mark)
}

/// Bob's device sends in the talk's channel, as its `counter`th line there, a `refused` receipt
/// with [`REASON`] from his agent `agent` to claude of `msg_id`.
#[track_caller]
fn bob_refuses(agents: &mut Agents, (agent, counter): (&str, u64), msg_id: MessageId) {
    let receipt = Receipt {
        status: ReceiptStatus::Refused,
        messages: NonEmpty::new(msg_id),
        reason: Some(REASON.to_owned()),
        unknown: Unknown::default(),
    };
    let bob = &agents.talk.played[0].person.handle;
    let mut line = Line::new(&format!("{ACME}/{bob}/{agent}@{HOST}"), counter, "");
    line.body = Some(Body::Receipt(receipt));
    line.to = Some(agents.address(CLAUDE));
    agents.bob_sends(&line);
}

/// claude sends a message `--to` Bob's agent in the talk's channel, and one in the default
/// channel. Bob's device sends in the talk's channel a `refused` receipt of the first from another
/// of his agents, and one of the second from the agent it went to; Ada's `skald recv` shows
/// `refused` of neither. Bob's device then sends a `refused` receipt of the first from the agent it
/// went to, and Ada's runs show `refused` of it once, with its reason.
#[test]
fn refused_counts_only_from_the_to_agent_for_a_message_sent_in_the_receipt_s_channel() {
    let mut agents = Agents::new(
        [4_731_009_001, 4_731_009_002],
        ["ada-agent-gets-refusals", "bob-forges-refusals"],
    );
    agents.run_as(CLAUDE);
    let in_default = claude_sends(&agents, agents.talk.space.default, IN_DEFAULT);
    let in_channel = claude_sends(&agents, agents.talk.channel, IN_CHANNEL);
    bob_refuses(&mut agents, (BOB_OTHER, 1), in_channel);
    bob_refuses(&mut agents, (BOB_AGENT, 2), in_default);

    let shown = received(&agents.talk.ada.machine).shown();

    let forged = refused_lines(&shown, in_channel);
    assert!(forged.is_empty(), "from another agent: {forged:?}");
    let elsewhere = refused_lines(&shown, in_default);
    assert!(elsewhere.is_empty(), "in another channel: {elsewhere:?}");

    bob_refuses(&mut agents, (BOB_AGENT, 3), in_channel);

    let shown = shows_refused_once(&agents.talk.ada.machine, in_channel);
    let lines = refused_lines(&shown, in_channel);
    assert!(
        lines.iter().all(|line| line.contains(REASON)),
        "{REFUSED} without its reason: {lines:?}"
    );
    assert!(refused_lines(&shown, in_default).is_empty(), "{shown}");
}
