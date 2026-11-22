//! The sending device takes a `read` receipt only from the message's `to` agent, for a message it
//! sent in the receipt's channel; it ignores any other.

use skald_proto::{Body, ChannelId, MessageId, NonEmpty, Receipt, ReceiptStatus, Unknown};

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::kit::received;
use crate::receipts::{read_lines, shows_read_once};
use crate::talk::{Line, one_message_sent};
use crate::world::{ACME, AGENT, HOST};

/// Bob's agent claude's messages go to, and another of his agents.
const BOB_AGENT: &str = "claude-w8c4rd";
const BOB_OTHER: &str = "codex-j6y2tf";

/// What claude sends; none is anywhere else.
const IN_CHANNEL: &str = "receipt forgery test: bob's agent, review the patch";
const IN_DEFAULT: &str = "receipt forgery test: bob's agent, review the schema";

/// Runs `skald --agent claude send --to <bob>/<BOB_AGENT> --channel <channel> <text>` in
/// `~/work/acme`; fails unless it succeeds. Gives the `msg_id` of the message it sent.
#[track_caller]
fn claude_sends(agents: &Agents, channel: ChannelId, text: &str) -> MessageId {
    let talk = &agents.talk;
    let to = format!("{}/{BOB_AGENT}", talk.played[0].person.handle);
    let id = channel.to_string();
    let from = talk.front.sent().len();
    let args = [AGENT, CLAUDE, SEND, TO, &to, CHANNEL, &id, text];
    let sent = talk.ada.machine.start_in(&agents.folder, &args).finish();
    assert!(sent.success, "{sent:?}");
    one_message_sent(&talk.front, channel, from)
}

/// Bob's device sends in the talk's channel, as its `counter`th line there, a `read` receipt from
/// his agent `agent` to claude of `msg_id`.
#[track_caller]
fn bob_reports(agents: &mut Agents, (agent, counter): (&str, u64), msg_id: MessageId) {
    let bob = agents.talk.played[0].person.handle.to_string();
    let mut line = Line::new(&format!("{ACME}/{bob}/{agent}@{HOST}"), counter, "");
    line.to = Some(agents.address(CLAUDE));
    line.body = Some(Body::Receipt(Receipt {
        status: ReceiptStatus::Read,
        messages: NonEmpty::new(msg_id),
        reason: None,
        unknown: Unknown::default(),
    }));
    agents.bob_sends(&line);
}

/// claude sends a message `--to` Bob's agent in the talk's channel, and one in the default
/// channel. Bob's device sends in the talk's channel a `read` receipt of the first from another of
/// his agents, and one of the second from the agent it went to; Ada's `skald recv` shows `read` of
/// neither. Bob's device then sends a `read` receipt of the first from the agent it went to, and
/// Ada's runs show `read` of it once.
#[test]
fn read_counts_only_from_the_to_agent_for_a_message_sent_in_the_receipt_s_channel() {
    let mut agents = Agents::new(
        [4_155_008_001, 4_155_008_002],
        ["ada-agent-gets-forgeries", "bob-forges-receipts"],
    );
    agents.run_as(CLAUDE);
    let (channel, default) = (agents.talk.channel, agents.talk.space.default);
    let in_channel = claude_sends(&agents, channel, IN_CHANNEL);
    let in_default = claude_sends(&agents, default, IN_DEFAULT);
    bob_reports(&mut agents, (BOB_OTHER, 1), in_channel);
    bob_reports(&mut agents, (BOB_AGENT, 2), in_default);

    let shown = received(&agents.talk.ada.machine).shown();

    assert_eq!(
        read_lines(&shown, in_channel),
        0,
        "from another agent: {shown}"
    );
    assert_eq!(
        read_lines(&shown, in_default),
        0,
        "in another channel: {shown}"
    );

    bob_reports(&mut agents, (BOB_AGENT, 3), in_channel);

    shows_read_once(&agents.talk.ada.machine, in_channel);
}
