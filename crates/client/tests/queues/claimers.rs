//! A task to an agent of another account: `sched` names the device whose live presence value lists
//! the agent, not one whose value expired, and the task gets the 7-day `deadline`, as another
//! account's device may claim it.

use skald_proto::AgentState;

use crate::fixtures::{CHANNEL, TO};
use crate::queuing::{
    assert_week_after, default_queue, enqueued, opened, signing_of, the_one_task,
};
use crate::values::{Directory, VALID, status, uploads_until};
use crate::world::now;

/// Bob's agent the tasks are for, and his second machine's certificate `name`.
const BOB_AGENT: &str = "codex-k7m2wq";
const BOB_LAPTOP: &str = "bob-laptop";

/// What Ada enqueues; no text is anywhere else.
const FOR_BOB: &str = "claimer test: ada asks bob's codex to review the migration";
const FOR_LIVE: &str = "claimer test: ada asks bob's codex where its value is live";

/// Bob's device uploads a value listing his agent `codex-k7m2wq` available. In `~/work/acme`, the
/// person's `skald enqueue --channel <channel> --to <bob>/codex-k7m2wq <text>` puts a task whose
/// `sched` names Bob's device and has a `deadline` a week after its time of enqueue.
#[test]
fn addressee_on_another_account_s_device_gets_a_deadline_seven_days_after_enqueue() {
    let mut directory = Directory::new(
        [4_265_073_001, 4_265_073_002],
        ["ada-enqueues-for-bob-s-agent", "bob-s-agent-may-claim"],
    );
    let at = directory.at();
    let agent = directory.bob_agent(BOB_AGENT);
    let listed = status(&agent, AgentState::Available, now(), (None, None));
    directory.bob_uploads((at, 1), vec![listed]);
    let to = format!("{}/{BOB_AGENT}", directory.bob_handle());
    let bob_device = directory.bob().device();
    let (cloud, ada) = (directory.cloud(), &directory.agents.talk.ada);
    let queue = default_queue(cloud, ada, at);
    let signing = signing_of(cloud, ada);
    let channel = at.1.to_string();
    let before = now();

    enqueued(
        directory.machine(),
        (directory.folder(), None),
        &[CHANNEL, &channel, TO, &to, FOR_BOB],
    );

    let after = now();
    let stored = the_one_task(cloud, ada, at, queue);
    let sched = &stored.task.sched;
    assert_eq!(
        sched.device,
        Some(bob_device),
        "not Bob's device: {sched:?}"
    );
    let opened = opened(&mut directory.agents.talk, at, (queue, &stored), &signing);
    assert_week_after(
        stored.task.sched.deadline,
        opened.body.enqueued,
        (before, after),
    );
}

/// Bob's device uploads a value listing `codex-k7m2wq` whose `expires` passed a minute ago, and his
/// second machine, `bob-laptop`, a live one listing it. The person's `skald enqueue --channel
/// <channel> --to <bob>/codex-k7m2wq <text>` puts a task whose `sched` names the laptop's device.
#[test]
fn addressee_listed_by_a_live_and_an_expired_value_goes_to_the_live_device() {
    let mut directory = Directory::new(
        [4_265_074_001, 4_265_074_002],
        ["ada-enqueues-past-an-old-value", "bob-left-an-old-value"],
    );
    let mut laptop = directory.second_machine(true, BOB_LAPTOP);
    let at = directory.at();
    let agent = directory.bob_agent(BOB_AGENT);
    let expires = now() - 60;
    let expired = status(&agent, AgentState::Available, expires - VALID, (None, None));
    let talk = &mut directory.agents.talk;
    uploads_until(
        &talk.world.cloud,
        &mut talk.played[0],
        (at, 1),
        expires,
        vec![expired],
    );
    let live = status(&agent, AgentState::Available, now(), (None, None));
    directory.uploads(&mut laptop, (at, 1), vec![live]);
    let to = format!("{}/{BOB_AGENT}", directory.bob_handle());
    let (cloud, ada) = (directory.cloud(), &directory.agents.talk.ada);
    let queue = default_queue(cloud, ada, at);
    let channel = at.1.to_string();

    enqueued(
        directory.machine(),
        (directory.folder(), None),
        &[CHANNEL, &channel, TO, &to, FOR_LIVE],
    );

    let stored = the_one_task(cloud, ada, at, queue);
    let sched = &stored.task.sched;
    assert_eq!(
        sched.device,
        Some(laptop.device()),
        "not the laptop's device: {sched:?}"
    );
}
