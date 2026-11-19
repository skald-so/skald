//! Replies that send nothing and say why: to a message the device does not keep, as an agent to one
//! not handed over to it, and by a reference two kept messages share; a reference without its `#`
//! works.

use std::collections::HashMap;

use skald_proto::MessageId;

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::kit::received;
use crate::lines::{SHORTEST, agent_line_of, reference};
use crate::replying::{
    BOB_CLAUDE, Writes, agent_of, assert_bob_never_reads, assert_refused, assert_reply, bob_reads,
    bob_writes, handles, replies, the_one,
};
use crate::world::ACME;

/// What Bob's device says, as Bob's claude; none is anywhere else.
const TO_CLAUDE: &str = "refusal test: bob's claude asks ada's claude for the logs";
const TO_CODEX: &str = "refusal test: bob's claude asks ada's codex for the metrics";
const TWINS: [&str; 2] = [
    "refusal test: the first of two messages whose references start alike",
    "refusal test: the second of two messages whose references start alike",
];

/// What Ada's claude tries to send; none is anywhere else.
const TO_UNKNOWN_REF: &str = "refusal test: a reply to a reference the device does not keep";
const TO_UNKNOWN_ID: &str = "refusal test: a reply to a msg_id the device does not keep";
const TO_CODEX_S: &str = "refusal test: claude's reply to the message for codex";
const TO_SHARED: &str = "refusal test: a reply by the reference both messages share";
const WITHOUT_HASH: &str = "refusal test: a reply by a reference without its #";
const BY_LONGER: &str = "refusal test: a reply by the longer reference claude was shown";

/// Two new message IDs whose references share their first [`SHORTEST`] characters; gives them and
/// that prefix.
fn twins() -> ([MessageId; 2], String) {
    let mut seen: HashMap<String, MessageId> = HashMap::new();
    loop {
        let msg_id = MessageId::generate();
        let prefix: String = reference(msg_id).chars().take(SHORTEST).collect();
        match seen.get(&prefix) {
            Some(earlier) if *earlier != msg_id => return ([*earlier, msg_id], prefix),
            _ => {
                seen.insert(prefix, msg_id);
            }
        }
    }
}

/// Ada's device has run as claude and codex in `~/work/acme`. Bob's device says one thing to Ada's
/// claude and one to Ada's codex in the talk's channel, as Bob's claude; the person's `skald recv`
/// shows both, codex's shows its own, claude's its own. As claude, `skald reply` to a reference no
/// kept message has, to a `msg_id` the device does not keep, and to the `msg_id` of codex's message
/// each fails, says why and sends nothing. Then claude's reply by its message's reference without
/// the `#` succeeds and reaches Bob's device as a reply to that message.
#[test]
fn reply_to_an_unknown_message_or_to_another_agent_s_sends_nothing_and_says_why() {
    let mut agents = Agents::new(
        [4_656_021_001, 4_656_021_002],
        ["ada-agent-replies-to-its-own", "bob-writes-to-both-agents"],
    );
    agents.run_as(CLAUDE);
    agents.run_as(CODEX);
    let (bob, ada) = handles(&agents.talk);
    let bob_claude = agent_of(&bob, ACME, BOB_CLAUDE);
    let at = agents.talk.at();
    let mut said = Vec::new();
    for (counter, (agent, text)) in [(CLAUDE, TO_CLAUDE), (CODEX, TO_CODEX)].iter().enumerate() {
        let to = agent_of(&ada, ACME, agent);
        let writes = Writes::new(&bob_claude, Some(&to), counter as u64 + 1, text);
        said.push(bob_writes(&mut agents.talk, at, &writes.hops(1)));
    }
    let person = received(&agents.talk.ada.machine).shown();
    assert!(person.contains(TO_CODEX), "{person}");
    assert!(agents.run_as(CODEX).shown().contains(TO_CODEX));
    let claude = agents.run_as(CLAUDE).shown();
    let short = agent_line_of(&claude, TO_CLAUDE).reference;
    let unknown = format!("#{}", &reference(MessageId::generate())[..SHORTEST + 2]);

    for (message, text, what) in [
        (unknown, TO_UNKNOWN_REF, "an unknown reference"),
        (
            MessageId::generate().to_string(),
            TO_UNKNOWN_ID,
            "an unknown msg_id",
        ),
        (said[1].to_string(), TO_CODEX_S, "codex's message"),
    ] {
        assert_refused(&replies(&agents, Some(CLAUDE), &message, text), &[], what);
    }
    let channels = [agents.talk.channel, agents.talk.space.default];
    assert_bob_never_reads(
        &mut agents.talk,
        &channels,
        &[TO_UNKNOWN_REF, TO_UNKNOWN_ID, TO_CODEX_S],
    );

    let replied = replies(&agents, Some(CLAUDE), &short, WITHOUT_HASH);

    assert!(replied.success, "{replied:?}");
    let read = bob_reads(&mut agents.talk, at.1);
    let (_, content) = the_one(&read, WITHOUT_HASH);
    let from = agent_of(&ada, ACME, CLAUDE);
    assert_reply(&content, (&from, &bob_claude), said[0], 2);
}

/// Ada's device has run as claude in `~/work/acme`. Bob's device says two things to Ada's claude in
/// the talk's channel as Bob's claude, in envelopes whose `msg_id`s have references that start
/// alike; claude's `skald recv` shows both. As claude, `skald reply` by the first characters both
/// references share fails, says why and sends nothing; by the longer reference claude was shown for
/// the first, it succeeds and reaches Bob's device as a reply to the first.
#[test]
fn reply_by_a_reference_two_kept_messages_share_sends_nothing_and_says_why() {
    let mut agents = Agents::new(
        [4_656_022_001, 4_656_022_002],
        ["ada-agent-sees-alike-refs", "bob-sends-alike-refs"],
    );
    agents.run_as(CLAUDE);
    let (bob, ada) = handles(&agents.talk);
    let bob_claude = agent_of(&bob, ACME, BOB_CLAUDE);
    let ada_claude = agent_of(&ada, ACME, CLAUDE);
    let (ids, shared) = twins();
    let at = agents.talk.at();
    for (counter, (msg_id, text)) in ids.into_iter().zip(TWINS).enumerate() {
        let mut writes = Writes::new(&bob_claude, Some(&ada_claude), counter as u64 + 1, text);
        writes.msg_id = Some(msg_id);
        bob_writes(&mut agents.talk, at, &writes.hops(1));
    }
    let claude = agents.run_as(CLAUDE).shown();
    let longer = agent_line_of(&claude, TWINS[0]).reference;
    assert!(longer.len() > SHORTEST, "#{longer} is no longer: {claude}");

    let refused = replies(&agents, Some(CLAUDE), &format!("#{shared}"), TO_SHARED);

    assert_refused(&refused, &[], "a shared reference");
    let channels = [agents.talk.channel, agents.talk.space.default];
    assert_bob_never_reads(&mut agents.talk, &channels, &[TO_SHARED]);

    let replied = replies(&agents, Some(CLAUDE), &format!("#{longer}"), BY_LONGER);

    assert!(replied.success, "{replied:?}");
    let read = bob_reads(&mut agents.talk, at.1);
    let (_, content) = the_one(&read, BY_LONGER);
    assert_reply(&content, (&ada_claude, &bob_claude), ids[0], 2);
}
