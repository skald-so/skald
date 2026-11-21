//! The limit on `hops` lets an agent's task at the limit through, and the person's next run warns
//! once of a task it stopped, naming the channel and the addressee, as of a send.

use skald_proto::ChannelId;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{CHANNEL, TO};
use crate::kit::received;
use crate::machine::Machine;
use crate::queuing::{
    Sending, assert_none_uploaded, enqueue, enqueued, opened, referred, sending, tasks,
};
use crate::replying::{BOB_CLAUDE, Writes, agent_of, bob_writes, handles};
use crate::world::ACME;

/// What Bob's claude says to Ada's claude, and what claude enqueues; no text is anywhere else.
const AT_NINE: &str = "task limit test: bob's claude asks ada's claude with 9 hops";
const AT_TEN: &str = "task limit test: bob's claude asks ada's claude with 10 hops";
const AT_LIMIT: &str = "task limit test: claude enqueues at hop 10";
const PAST_LIMIT: &str = "task limit test: claude would enqueue for bob's claude at hop 11";

/// How many lines of the person's next `skald recv` on `machine` warn of `channel` and
/// `addressee`.
fn warnings_of(machine: &Machine, channel: ChannelId, addressee: &str) -> usize {
    let shown = received(machine).shown();
    let channel = channel.to_string();
    (shown.lines())
        .filter(|line| line.contains("warning") && line.contains(&channel))
        .filter(|line| line.contains(addressee))
        .count()
}

/// Bob's claude says `line` to Ada's claude in the talk's channel, as Bob's `n`th message, with
/// `hops`, and claude's `skald recv` in `~/work/acme` takes it.
fn hand_over(agents: &mut Agents, (n, hops, line): (u64, u64, &str)) {
    let (bob, ada) = handles(&agents.talk);
    let from = agent_of(&bob, ACME, BOB_CLAUDE);
    let to = agent_of(&ada, ACME, CLAUDE);
    let at = agents.talk.at();
    bob_writes(
        &mut agents.talk,
        at,
        &Writes::new(&from, Some(&to), n, line).hops(hops),
    );
    agents.run_as(CLAUDE);
}

/// Bob's claude says something to Ada's claude with `hops` 9, which claude's `skald recv` in
/// `~/work/acme` takes: claude's `skald enqueue --channel <channel> <text>` puts a task with
/// `hops` 10. Bob's claude then says something with `hops` 10, which claude takes: claude's `skald
/// enqueue --channel <channel> --to <bob>/<claude> <text>` uploads no task. The person's next
/// `skald recv` warns once of it, naming the channel and Bob's claude, and the run after it does
/// not.
#[test]
fn agent_s_task_at_ten_hops_is_stored_and_the_person_is_warned_once_of_one_past_it() {
    let mut agents = Agents::new(
        [4_265_072_001, 4_265_072_002],
        ["ada-enqueues-at-the-limit", "bob-loops-to-the-limit"],
    );
    agents.run_as(CLAUDE);
    let ada_s: Sending = sending(&agents.talk);
    let (at, queue) = (ada_s.at, ada_s.queue);
    let to = format!("{}/{BOB_CLAUDE}", handles(&agents.talk).0);
    hand_over(&mut agents, (1, 9, AT_NINE));
    let folder = agents.folder.clone();
    let claude = (folder.as_path(), Some(CLAUDE));

    let ran = enqueued(
        &agents.talk.ada.machine,
        claude,
        &[CHANNEL, &ada_s.channel, AT_LIMIT],
    );

    let held = tasks(agents.talk.cloud(), &agents.talk.ada, at, queue);
    let stored = referred(&held, &ran);
    let content = opened(&mut agents.talk, at, (queue, &stored), &ada_s.signing)
        .body
        .content;
    assert_eq!(content.hops, 10, "{content:?}");

    hand_over(&mut agents, (2, 10, AT_TEN));
    let since = agents.talk.cloud().relay.mark();

    let refused = enqueue(
        &agents.talk.ada.machine,
        claude,
        &[CHANNEL, &ada_s.channel, TO, &to, PAST_LIMIT],
    );

    assert!(!refused.success, "enqueued past the limit: {refused:?}");
    assert_none_uploaded(agents.talk.cloud(), since, "the task at hop 11");
    let machine = &agents.talk.ada.machine;
    assert_eq!(warnings_of(machine, at.1, &to), 1, "not warned once");
    assert_eq!(warnings_of(machine, at.1, &to), 0, "warned again");
}
