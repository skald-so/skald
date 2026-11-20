//! A person's message starts the count again: the person's messages carry `hops` 0, and an agent's
//! reply to one carries 1, the stopped agent's too.

use crate::agents::CLAUDE;
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::kit::received;
use crate::loops::{Chain, LIMIT, Side, Way, assert_sent, assert_stopped, texts};
use crate::replying::{REPLY, assert_reply, run_in, the_one};

/// What the people say and the agents answer them; none is anywhere else.
const A_PERSON_ANSWERS: &str = "hop limit test: ada answers claude's last message herself";
const B_ANSWERS_PERSON: &str = "hop limit test: claude answers ada's own message";
const B_PERSON_ASKS: &str = "hop limit test: ada asks the stopped agent from her first machine";
const A_ANSWERS_PERSON: &str = "hop limit test: the stopped agent answers ada";

/// A's and B's agents reply to each other with the reply tool until A's reply at hop 11 is
/// refused. A's person's run shows B's last message, with `hops` 10, and A's person replies to it
/// by its `msg_id`: Bob's played device reads the reply from Ada's person address to claude with
/// `hops` 0. B's agent takes it and replies with the reply tool: Bob's device reads that reply to
/// Ada's person address with `hops` 1. B's person sends `--to` A's agent: `hops` 0; A's agent,
/// the stopped one, takes it and replies with the reply tool: the reply is sent, with `hops` 1.
#[test]
fn person_s_message_starts_the_count_again() {
    let mut chain = Chain::new(
        Way::ReplyTool,
        [4_811_004_001, 4_811_004_002],
        ["ada-steps-into-the-loop", "bob-reads-the-reset-hops"],
    );
    let texts = texts(Way::ReplyTool);
    let last = &texts[LIMIT as usize];
    let taken = chain.answer_each_other(&texts[..=LIMIT as usize]);
    let refused = chain.answer(Side::A, (&taken, last), &texts[LIMIT as usize + 1]);
    assert_stopped(&refused, "the reply at hop 11");
    let (b_last, _) = the_one(&chain.c_reads(), last);
    let talk = &chain.roles.senders.agents.talk;
    let (ada, claude) = (talk.address(&talk.ada), chain.roles.full(CLAUDE));

    let shown = received(chain.machine(Side::A)).shown();
    assert!(
        shown.contains(last.as_str()),
        "A's person not shown {last:?}: {shown}"
    );
    let a_folder = chain.folder(Side::A);
    let answered = run_in(
        chain.machine(Side::A),
        (&a_folder, None),
        REPLY,
        &[&b_last.to_string(), A_PERSON_ANSWERS],
    );

    assert!(answered.success, "{answered:?}");
    let (person_reply, content) = the_one(&chain.c_reads(), A_PERSON_ANSWERS);
    assert_reply(&content, (&ada, &claude), b_last, 0);
    let got = chain.take(Side::B);
    let said = chain.answer(Side::B, (&got, A_PERSON_ANSWERS), B_ANSWERS_PERSON);
    assert_sent(&said, B_ANSWERS_PERSON);
    let (_, content) = the_one(&chain.c_reads(), B_ANSWERS_PERSON);
    assert_reply(&content, (&claude, &ada), person_reply, 1);

    let (channel, a) = (chain.channel().to_string(), chain.short(Side::A));
    let b_folder = chain.folder(Side::B);
    let args = [TO, a.as_str(), CHANNEL, &channel, B_PERSON_ASKS];
    let asked = run_in(chain.machine(Side::B), (&b_folder, None), SEND, &args);

    assert!(asked.success, "{asked:?}");
    let (person_message, content) = the_one(&chain.c_reads(), B_PERSON_ASKS);
    assert_eq!(content.hops, 0, "the person's message: {content:?}");
    let got = chain.take(Side::A);
    let said = chain.answer(Side::A, (&got, B_PERSON_ASKS), A_ANSWERS_PERSON);
    assert_sent(&said, A_ANSWERS_PERSON);
    let (_, content) = the_one(&chain.c_reads(), A_ANSWERS_PERSON);
    let a_full = chain.roles.full(Chain::agent(Side::A));
    assert_reply(&content, (&a_full, &ada), person_message, 1);
}
