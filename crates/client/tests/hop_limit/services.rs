//! A member service agent counts what its runs show as handed over to it: its `skald send` carries
//! one more than the highest `hops` its `skald recv` showed since its last message, or 0, and over
//! 10 sends nothing.

use crate::loops::{Said, assert_stopped};
use crate::member::Bot;
use crate::replying::{
    BOB_CLAUDE, Writes, agent_of, assert_bob_never_reads, bob_reads, bob_writes, handles, the_one,
};
use crate::world::ACME;

/// What the agent says; none is anywhere else.
const FIRST: &str = "hop limit member bot test: deploy-bot's first line, with nothing shown";
const AT_TEN: &str = "hop limit member bot test: deploy-bot answers the line with 9 hops";
const OVER: &str = "hop limit member bot test: deploy-bot would answer at hop 11";

/// What Bob's claude says to everyone; none is anywhere else.
const NINE: &str = "hop limit member bot test: bob's claude asks with 9 hops";
const TEN: &str = "hop limit member bot test: bob's claude asks again with 10 hops";

/// Ada's machine creates `ci/deploy-bot` as a member of the channel, and it gets into the group.
/// Its first `skald send` carries `hops` 0. Bob's claude says a line to everyone with `hops` 9; the
/// agent's `skald recv` shows it, and its next send carries 10. Bob's claude says a line with
/// `hops` 10; once the agent's run shows it, its send fails, says why, and Bob's device never
/// reads it.
#[test]
fn member_service_agent_s_send_counts_what_its_runs_showed_and_stops_at_ten_hops() {
    let mut bot = Bot::created(
        [4_811_011_001, 4_811_011_002],
        ["ada-member-bot-counts-hops", "bob-pushes-the-bot-to-ten"],
    );
    bot.brought_in();
    let (bob, _) = handles(&bot.talk);
    let bob_claude = agent_of(&bob, ACME, BOB_CLAUDE);
    let (at, channel) = (bot.talk.at(), bot.channel());

    bot.says(FIRST);
    assert_eq!(hops_of(&mut bot, FIRST), 0, "{FIRST:?}");
    bob_writes(
        &mut bot.talk,
        at,
        &Writes::new(&bob_claude, None, 1, NINE).hops(9),
    );
    let got = bot.runner.received();
    assert!(got.shown().contains(NINE), "{NINE:?} not shown: {got:?}");
    bot.says(AT_TEN);
    assert_eq!(hops_of(&mut bot, AT_TEN), 10, "{AT_TEN:?}");
    bob_writes(
        &mut bot.talk,
        at,
        &Writes::new(&bob_claude, None, 2, TEN).hops(10),
    );
    let got = bot.runner.received();
    assert!(got.shown().contains(TEN), "{TEN:?} not shown: {got:?}");

    let refused: Said = bot.runner.send(channel, OVER).into();

    assert_stopped(&refused, "the agent's send at hop 11");
    assert_bob_never_reads(&mut bot.talk, &[channel], &[OVER]);
}

/// The `hops` of the one message Bob's device read in the channel with `text`.
#[track_caller]
fn hops_of(bot: &mut Bot, text: &str) -> u64 {
    let channel = bot.channel();
    the_one(&bob_reads(&mut bot.talk, channel), text).1.hops
}
