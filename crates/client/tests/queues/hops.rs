//! A task's `hops` as a send has them (`docs/protocol/README.md`, "Content"): an agent's one more
//! than the highest `hops` of the messages handed to it since the device last sent as it, the
//! person's 0; over 10, nothing is enqueued.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::CHANNEL;
use crate::queuing::{
    Sending, Stored, assert_none_uploaded, assert_refused_naming, enqueue, enqueued, opened,
    referred, sending, tasks,
};
use crate::replying::{BOB_CLAUDE, Writes, agent_of, bob_writes, handles};
use crate::world::ACME;

/// What Bob's claude says to Ada's claude, and what Ada's claude and Ada enqueue; no text is
/// anywhere else.
const AT_THREE: &str = "task hops test: bob's claude asks ada's claude with 3 hops";
const AT_TEN: &str = "task hops test: bob's claude asks ada's claude with 10 hops";
const AFTER_THREE: &str = "task hops test: claude enqueues after a message with 3 hops";
const AFTER_TEN: &str = "task hops test: claude would enqueue at hop 11";
const BY_PERSON: &str = "task hops test: ada enqueues as the person";

/// The highest `hops` an agent's content carries, and what a refusal of it names beside it.
const LIMIT: &str = "10";
const PERSON: &str = "person";

/// Bob's claude says something to Ada's claude in the channel with `hops` 3, and claude's `skald
/// recv` in `~/work/acme` takes it: claude's `skald enqueue --channel <channel> <text>` puts a task
/// whose sealed content has `hops` 4. Bob's claude then says something with `hops` 10, which
/// claude takes: claude's next `skald enqueue` fails naming `10` and `person`, and uploads no
/// task. The person's `skald enqueue` puts a task with `hops` 0.
#[test]
fn agent_s_task_carries_the_hops_of_a_send_and_stops_at_the_limit() {
    let mut agents = Agents::new(
        [4_265_061_001, 4_265_061_002],
        ["ada-counts-task-hops", "bob-loops-with-tasks"],
    );
    agents.run_as(CLAUDE);
    let (bob, ada_handle) = handles(&agents.talk);
    let bob_claude = agent_of(&bob, ACME, BOB_CLAUDE);
    let ada_claude = agent_of(&ada_handle, ACME, CLAUDE);
    let Sending {
        at,
        channel,
        queue,
        signing,
        ..
    } = sending(&agents.talk);
    let writes = Writes::new(&bob_claude, Some(&ada_claude), 1, AT_THREE).hops(3);
    bob_writes(&mut agents.talk, at, &writes);
    let taken = agents.run_as(CLAUDE).stdout;
    assert!(
        taken.contains(AT_THREE),
        "{AT_THREE:?} not handed over: {taken}"
    );
    let claude = (agents.folder.as_path(), Some(CLAUDE));

    let ran = enqueued(
        &agents.talk.ada.machine,
        claude,
        &[CHANNEL, &channel, AFTER_THREE],
    );

    let held = tasks(agents.talk.cloud(), &agents.talk.ada, at, queue);
    let stored: Stored = referred(&held, &ran);
    let content = opened(&mut agents.talk, at, (queue, &stored), &signing)
        .body
        .content;
    assert_eq!(content.hops, 4, "{content:?}");

    let writes = Writes::new(&bob_claude, Some(&ada_claude), 2, AT_TEN).hops(10);
    bob_writes(&mut agents.talk, at, &writes);
    let taken = agents.run_as(CLAUDE).stdout;
    assert!(
        taken.contains(AT_TEN),
        "{AT_TEN:?} not handed over: {taken}"
    );
    let since = agents.talk.cloud().relay.mark();

    let refused = enqueue(
        &agents.talk.ada.machine,
        claude,
        &[CHANNEL, &channel, AFTER_TEN],
    );

    assert_refused_naming(&refused, &[LIMIT, PERSON], "the task at hop 11");
    assert_none_uploaded(agents.talk.cloud(), since, "the task at hop 11");

    let person = (agents.folder.as_path(), None);
    let ran = enqueued(
        &agents.talk.ada.machine,
        person,
        &[CHANNEL, &channel, BY_PERSON],
    );

    let held = tasks(agents.talk.cloud(), &agents.talk.ada, at, queue);
    assert_eq!(held.len(), 2, "not two tasks: {held:?}");
    let stored = referred(&held, &ran);
    let content = opened(&mut agents.talk, at, (queue, &stored), &signing)
        .body
        .content;
    assert_eq!(content.hops, 0, "{content:?}");
}
