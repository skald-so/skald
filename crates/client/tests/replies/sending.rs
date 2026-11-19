//! What a reply sends: in the original message's channel, `to` its `from`, a person's address
//! included, `reply_to` its `msg_id`, and `hops` one more than the original's from an agent, 0 from
//! the person.

use crate::agents::{Agents, CLAUDE};
use crate::kit::received;
use crate::lines::{agent_line_of, assert_says_sent};
use crate::replying::{
    BOB_CLAUDE, Writes, agent_of, assert_reply, bob_reads, bob_writes, handles, of_text, replies,
    the_one,
};
use crate::world::ACME;

/// What Bob's device says to Ada's claude, and what Ada's machine answers; none is anywhere else.
const FROM_AGENT: &str = "reply test: bob's claude asks ada's claude for the schema";
const FROM_PERSON: &str = "reply test: bob asks ada's claude for the roadmap";
const TO_AGENT: &str = "reply test: ada's claude sends bob's claude the schema";
const TO_PERSON: &str = "reply test: ada's claude sends bob the roadmap";
const ASKED: &str = "reply test: bob's claude asks for the budget";
const PERSON_ANSWERS: &str = "reply test: ada herself answers about the budget";

/// The `hops` of the message Bob's claude sends.
const HOPS: u64 = 2;

/// Ada's device has run as claude in `~/work/acme`. Bob's device says two things to Ada's claude in
/// the talk's channel, not `acme`'s default one: one as Bob's claude with `hops` [`HOPS`], one as
/// Bob. claude's `skald recv` shows both; claude replies to each by the reference it showed. Each
/// reply prints `sent #<ref>` of itself, and Bob's device reads each in the talk's channel, from
/// Ada's claude, `to` the original's `from`, Bob's claude or Bob, with `reply_to` the original and
/// `hops` one more than the original's; none in the default channel.
#[test]
fn agent_s_reply_goes_in_the_message_s_channel_to_its_author_with_one_more_hop() {
    let mut agents = Agents::new(
        [4_656_011_001, 4_656_011_002],
        ["ada-agent-replies", "bob-asks-ada-agent"],
    );
    agents.run_as(CLAUDE);
    let (bob, ada) = handles(&agents.talk);
    let (bob_claude, bob_person) = (agent_of(&bob, ACME, BOB_CLAUDE), agents.bob());
    let ada_claude = agent_of(&ada, ACME, CLAUDE);
    let at = agents.talk.at();
    let from_agent = Writes::new(&bob_claude, Some(&ada_claude), 1, FROM_AGENT).hops(HOPS);
    let from_agent = bob_writes(&mut agents.talk, at, &from_agent);
    let from_person = Writes::new(&bob_person, Some(&ada_claude), 2, FROM_PERSON);
    let from_person = bob_writes(&mut agents.talk, at, &from_person);
    let claude = agents.run_as(CLAUDE).shown();
    let refs = [FROM_AGENT, FROM_PERSON].map(|text| agent_line_of(&claude, text).reference);

    let replied = [(&refs[0], TO_AGENT), (&refs[1], TO_PERSON)]
        .map(|(short, text)| replies(&agents, Some(CLAUDE), &format!("#{short}"), text));

    for one in &replied {
        assert!(one.success, "{one:?}");
    }
    let channel = agents.talk.channel;
    let read = bob_reads(&mut agents.talk, channel);
    let to_agent = the_one(&read, TO_AGENT);
    assert_reply(
        &to_agent.1,
        (&ada_claude, &bob_claude),
        from_agent,
        HOPS + 1,
    );
    let to_person = the_one(&read, TO_PERSON);
    assert_reply(&to_person.1, (&ada_claude, &bob_person), from_person, 1);
    assert_says_sent(&replied[0].shown(), to_agent.0);
    assert_says_sent(&replied[1].shown(), to_person.0);
    let default = agents.talk.space.default;
    let elsewhere = bob_reads(&mut agents.talk, default);
    for text in [TO_AGENT, TO_PERSON] {
        assert!(
            of_text(&elsewhere, text).is_empty(),
            "{text:?} in {default}"
        );
    }
}

/// Bob's device says something to Ada's claude in the talk's channel as Bob's claude, with `hops`
/// [`HOPS`]; the person's `skald recv` shows it. The person's `skald reply` to its `msg_id`
/// succeeds, and Bob's device reads the reply in the talk's channel, from Ada's person address,
/// `to` Bob's claude, with `reply_to` the original and `hops` 0.
#[test]
fn person_s_reply_goes_to_the_agent_with_no_hops() {
    let mut agents = Agents::new(
        [4_656_012_001, 4_656_012_002],
        ["ada-replies-herself", "bob-agent-asks-ada"],
    );
    let (bob, ada) = handles(&agents.talk);
    let bob_claude = agent_of(&bob, ACME, BOB_CLAUDE);
    let at = agents.talk.at();
    let asked = Writes::new(&bob_claude, Some(&agent_of(&ada, ACME, CLAUDE)), 1, ASKED);
    let asked = bob_writes(&mut agents.talk, at, &asked.hops(HOPS));
    let person = received(&agents.talk.ada.machine).shown();
    assert!(person.contains(ASKED), "{person}");

    let replied = replies(&agents, None, &asked.to_string(), PERSON_ANSWERS);

    assert!(replied.success, "{replied:?}");
    let channel = agents.talk.channel;
    let read = bob_reads(&mut agents.talk, channel);
    let (_, content) = the_one(&read, PERSON_ANSWERS);
    let ada_person = agents.talk.address(&agents.talk.ada);
    assert_reply(&content, (&ada_person, &bob_claude), asked, 0);
}
