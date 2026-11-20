//! An agent's `skald send`: only `sent #<ref>` once the server has stored the message, and the full
//! report when it refused it.

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE};
use crate::front::Answers;
use crate::lines::{assert_says_sent, says_sent};
use crate::machine::Finished;
use crate::said::send_as;
use crate::talk::assert_read;

/// What claude sends; none is anywhere else.
const STORED: &str = "send test: the changelog is updated";
const REFUSED: &str = "send test: this one the server refuses";

/// The problem the front refuses a message with, in place of the server.
const REFUSAL: &str = "not-a-member";

/// Runs `skald --agent <CLAUDE> send --all --channel <channel> <text>` in `~/work/acme`; fails
/// unless it sent one message. Gives the run and that message's `msg_id`.
#[track_caller]
fn claude_sends(agents: &Agents, text: &str) -> (Finished, MessageId) {
    let (finished, sent) = send_as(agents, (CLAUDE, &[]), text);
    let [envelope] = sent.as_slice() else {
        panic!("not one message sent: {sent:?} {finished:?}");
    };
    (finished, envelope.envelope().header.msg_id)
}

/// As claude in `~/work/acme`, `skald send` to the channel prints one line, `sent #<ref>`, the
/// reference of the message's `msg_id`, and nothing else; Bob's device reads the message from
/// claude's address. The front then refuses each message in place of the server: claude's next
/// `skald send` fails and prints the full report, with the message's ID, the channel's ID and the
/// problem, and no line says `sent #`.
#[test]
fn agent_s_send_prints_only_sent_when_stored_and_the_full_report_when_refused() {
    let mut agents = Agents::new(
        [4_149_031_001, 4_149_031_002],
        ["ada-agent-sends-quietly", "bob-reads-the-quiet-send"],
    );

    let (stored, id) = claude_sends(&agents, STORED);

    assert!(stored.success, "{stored:?}");
    assert_says_sent(&stored.shown(), id);
    let address = agents.address(CLAUDE);
    let talk = &mut agents.talk;
    talk.played[0].take_in(&talk.world.cloud);
    let sender = (talk.ada.device, address.as_str());
    assert_read(&talk.played[0], talk.channel, sender, &[(id, 1, STORED)]);

    talk.front.answer(Answers::Refuse(REFUSAL.to_owned()));
    let (refused, id) = claude_sends(&agents, REFUSED);

    assert!(!refused.success, "{refused:?}");
    let shown = refused.shown();
    let channel = agents.talk.channel.to_string();
    for part in [id.to_string(), channel, REFUSAL.to_owned()] {
        assert!(shown.contains(&part), "no {part}: {refused:?}");
    }
    assert!(!says_sent(&shown), "{refused:?}");
}
