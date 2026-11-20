//! Two agents answering each other, by the reply tool, the send tool or `skald send --to`, stop at
//! 10 hops without a human: the hops rise by one per answer, the one that would carry 11 sends
//! nothing and says why, and its person's next run warns of it once.

use crate::loops::{
    Chain, LIMIT, Side, Way, assert_person_warned_once, assert_stopped, person_warned, texts,
};

/// A's and B's agents answer each other `way`: B sends the first message, and each answer is to
/// the other's last one, up to B's with `hops` 10; Bob's played device reads the hops of each,
/// rising by one from 0. A's person's run warns of nothing. A takes B's last message and answers
/// it: the answer fails, says that the agents answered each other 10 times and that a person's
/// message starts the count again, and no device reads it; B's agent never gets it. A's person's
/// next run warns once, with the channel and B's agent, and the run after it does not. Gives the
/// chain and its texts.
#[track_caller]
fn stops_at_the_limit(way: Way, ids: [u64; 2], logins: [&str; 2]) -> (Chain, Vec<String>) {
    let mut chain = Chain::new(way, ids, logins);
    let texts = texts(way);
    let within = &texts[..=LIMIT as usize];
    let last = chain.answer_each_other(within);
    chain.assert_hops_rise(within);
    let (channel, b) = (chain.channel(), chain.short(Side::B));
    let before = person_warned(chain.machine(Side::A), channel, &b);
    assert_eq!(before, 0, "warned before the limit");
    let over = &texts[LIMIT as usize + 1];

    let refused = chain.answer(Side::A, (&last, &texts[LIMIT as usize]), over);

    assert_stopped(&refused, "the answer at hop 11");
    chain.assert_never_sent(&[over]);
    let got = chain.take(Side::B);
    assert!(!got.contains(over.as_str()), "B got {over:?}: {got}");
    assert_person_warned_once(chain.machine(Side::A), channel, &b);
    (chain, texts)
}

/// As [`stops_at_the_limit`], by the reply tool, after B's first message by the send tool.
#[test]
fn agents_replying_to_each_other_stop_at_ten_hops_and_the_person_is_warned_once() {
    stops_at_the_limit(
        Way::ReplyTool,
        [4_811_001_001, 4_811_001_002],
        ["ada-agents-reply-in-a-loop", "bob-reads-the-reply-hops"],
    );
}

/// What A sends after its send is refused; it is nowhere else.
const AFTER: &str = "hop limit test: A sends again after the refused send";

/// As [`stops_at_the_limit`], by the send tool `to` the other. A's next send, after the refused
/// one, is refused too and sends nothing.
#[test]
fn agents_sending_to_each_other_with_the_send_tool_stop_at_ten_hops() {
    let (mut chain, _) = stops_at_the_limit(
        Way::SendTool,
        [4_811_002_001, 4_811_002_002],
        ["ada-agents-send-in-a-loop", "bob-reads-the-tool-hops"],
    );

    let again = chain.send(Side::A, AFTER);

    assert_stopped(&again, "the send after the refused one");
    chain.assert_never_sent(&[AFTER]);
}

/// As [`stops_at_the_limit`], by `skald --agent <agent> send --to <the other>` after the agent's
/// `skald recv`. A's next `skald send`, after the refused one, is refused too and sends nothing.
#[test]
fn agents_sending_to_each_other_with_skald_send_stop_at_ten_hops() {
    let (mut chain, _) = stops_at_the_limit(
        Way::Cli,
        [4_811_003_001, 4_811_003_002],
        ["ada-agents-loop-in-the-shell", "bob-reads-the-shell-hops"],
    );

    let again = chain.send(Side::A, AFTER);

    assert_stopped(&again, "the send after the refused one");
    chain.assert_never_sent(&[AFTER]);
}
