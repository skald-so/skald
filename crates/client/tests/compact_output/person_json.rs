//! The person's `--json`: the messages a run sends and receives, with every field.

use crate::agents::Agents;
use crate::front::Answers;
use crate::kit::RECV;
use crate::lines::{JSON_FLAG, assert_fields, holds_text, json_objects, object_of};
use crate::talk::{BOB_HOST, Line, Talk, one_message_sent};

/// What Ada and Bob's device say; none is anywhere else.
const ASKED: &str = "person json test: ada asks for the release notes";
const ANSWERED: &str = "person json test: bob attached the release notes";

/// The server's answer to the person's message is lost, so it waits in the outbox; Bob's device
/// then says something in the channel. The person's `skald recv --json` sends the waiting message
/// and gives, each as a JSON object on a line of its own, that message, stored, with its reference
/// and channel, and Bob's message with every field.
#[test]
fn json_gives_the_person_the_messages_it_sends_and_receives() {
    let talk = Talk::new(
        &[4_149_061_001, 4_149_061_002],
        &["ada-reads-json-herself", "bob-answers-her-json"],
        true,
    );
    let mut agents = Agents::bound(talk);
    let (front, channel) = (&agents.talk.front, agents.talk.channel);
    let from = front.sent().len();
    front.answer(Answers::Lose);
    let lost = agents.talk.send(ASKED);
    assert!(!lost.success, "{lost:?}");
    let asked = one_message_sent(front, channel, from);
    front.answer(Answers::Forward);
    let bob = agents.bob();
    let answered = agents.bob_sends(&Line::new(&bob, 1, ANSWERED));

    let got = (agents.talk.ada.machine).start(&[RECV, JSON_FLAG]).finish();

    assert!(got.success, "{got:?}");
    let objects = json_objects(&got.stdout);
    let said = object_of(&objects, asked, "envelopes");
    assert_fields(said, &[("channel", channel.to_string())]);
    assert!(holds_text(said, "stored"), "not stored: {said}");
    let message = object_of(&objects, answered, "text");
    let talk = &agents.talk;
    let fields = [
        ("channel", channel.to_string()),
        ("from", bob),
        ("device", talk.played[0].device().to_string()),
        ("device_name", BOB_HOST.to_owned()),
        ("text", ANSWERED.to_owned()),
    ];
    assert_fields(message, &fields);
}
