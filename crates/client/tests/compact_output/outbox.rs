//! An agent's `skald send` that also sends what waits in the outbox reports only its own message.

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE};
use crate::front::Answers;
use crate::lines::assert_says_sent;
use crate::said::send_as;
use crate::talk::one_message_sent;

/// What the person and claude send; none is anywhere else.
const PERSONS: &str = "outbox test: the person's message waits in the outbox";
const CLAUDES: &str = "outbox test: claude's message goes out after it";

/// The server's answer to the person's `skald send` is lost, so the message waits in the outbox.
/// As claude in `~/work/acme`, `skald send` sends it, then its own: it prints `sent #<ref>` of its
/// own message only.
#[test]
fn agent_s_send_reports_only_its_own_messages() {
    let agents = Agents::new(
        [4_149_081_001, 4_149_081_002],
        ["ada-leaves-a-message-waiting", "bob-gets-both-at-once"],
    );
    let talk = &agents.talk;
    talk.front.answer(Answers::Lose);
    let from = talk.front.sent().len();
    let lost = talk.send(PERSONS);
    assert!(!lost.success, "{lost:?}");
    let waiting = one_message_sent(&talk.front, talk.channel, from);
    talk.front.answer(Answers::Forward);

    let (stored, sent) = send_as(&agents, (CLAUDE, &[]), CLAUDES);

    assert!(stored.success, "{stored:?}");
    let ids: Vec<MessageId> = (sent.iter())
        .map(|envelope| envelope.envelope().header.msg_id)
        .collect();
    let [person, own] = ids.as_slice() else {
        panic!("not two messages sent: {ids:?} {stored:?}");
    };
    assert_eq!(*person, waiting, "{stored:?}");
    assert_says_sent(&stored.shown(), *own);
}
