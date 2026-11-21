//! A task's addressee: `--to` an agent puts it in the content's `to`, and in `sched` the one
//! device whose valid presence value lists it (`docs/protocol/agents.md`, "Presence"); with none
//! or several, nothing is enqueued, saying why.

use crate::agents::{CLAUDE, run_as_in};
use crate::fixtures::{CHANNEL, TO};
use crate::queuing::{
    Sending, assert_none_uploaded, assert_refused_naming, assert_says_reference, assert_task,
    enqueue, enqueued, opened, sending, tasks, the_one_task,
};
use crate::senders::{LAPTOP, LAPTOP_AGENT, Senders};
use crate::talk::ADA_HOST;
use crate::world::now;

/// What the tests enqueue; no text is anywhere else.
const FOR_LAPTOP: &str = "addressee test: claude asks the laptop's codex to profile the parser";
const FOR_NOBODY: &str = "addressee test: ada asks an agent no device lists";
const FOR_TWO: &str = "addressee test: ada asks claude, whom two devices list";

/// An agent of Ada's that no device lists.
const UNLISTED: &str = "gemini-p4r8vd";

/// The agent of Ada's second machine runs `skald recv` once in its `~/work/acme`, so that
/// machine's presence value lists it. As claude on her first machine, `skald enqueue --channel
/// <channel> --to <ada>/<agent> <text>` puts a task whose `sched` names the second machine's
/// device and has no `deadline`, though Bob's account is in the channel: only that device of
/// Ada's account may claim it. Its sealed content is from claude, `to` the agent's full address.
#[test]
fn addressee_on_the_second_device_puts_that_device_in_sched_and_gets_no_deadline() {
    let mut senders = Senders::new(
        [4_265_051_001, 4_265_051_002],
        ["ada-addresses-her-laptop", "bob-sees-the-addressee"],
    );
    run_as_in(
        &senders.laptop.machine,
        &senders.laptop_folder,
        LAPTOP_AGENT,
    );
    let (to, full) = (
        format!("{}/{LAPTOP_AGENT}", senders.ada()),
        senders.agents.address(LAPTOP_AGENT),
    );
    let claude = senders.agents.address(CLAUDE);
    let laptop = senders.laptop.device;
    let Sending {
        at,
        channel,
        queue,
        device: ada_device,
        signing,
    } = sending(&senders.agents.talk);
    let (cloud, ada) = (senders.agents.talk.cloud(), &senders.agents.talk.ada);
    let before = now();

    let ran = enqueued(
        &ada.machine,
        (&senders.agents.folder, Some(CLAUDE)),
        &[CHANNEL, &channel, TO, &to, FOR_LAPTOP],
    );

    let after = now();
    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&ran, stored.task.task);
    let sched = &stored.task.sched;
    assert_eq!(
        sched.device,
        Some(laptop),
        "not the laptop's device: {sched:?}"
    );
    assert_eq!(sched.deadline, None, "a deadline in {sched:?}");
    let opened = opened(&mut senders.agents.talk, at, (queue, &stored), &signing);
    assert_task(
        &opened,
        (at, queue, &stored),
        (ada_device, &claude, FOR_LAPTOP),
        (before, after),
    );
    let content = opened.content();
    assert_eq!(
        content.to.as_ref().map(ToString::to_string),
        Some(full),
        "{content:?}"
    );
}

/// Claude runs `skald recv` once on Ada's second machine too, so both her machines list claude.
/// The person's `skald enqueue --channel <channel> --to <ada>/gemini-p4r8vd <text>`, an agent no
/// value lists, fails naming it; `skald enqueue --channel <channel> --to <ada>/claude-… <text>`
/// fails naming both machines' devices. Neither uploads a task, and the server holds none.
#[test]
fn addressee_no_value_lists_or_two_devices_list_enqueues_nothing() {
    let senders = Senders::new(
        [4_265_052_001, 4_265_052_002],
        ["ada-addresses-nobody", "bob-sees-no-task"],
    );
    run_as_in(&senders.laptop.machine, &senders.laptop_folder, CLAUDE);
    let (unlisted, twice) = (
        format!("{}/{UNLISTED}", senders.ada()),
        format!("{}/{CLAUDE}", senders.ada()),
    );
    let Sending {
        at, channel, queue, ..
    } = sending(&senders.agents.talk);
    let (cloud, ada) = (senders.agents.talk.cloud(), &senders.agents.talk.ada);
    let person = (senders.agents.folder.as_path(), None);
    let since = cloud.relay.mark();

    let none = enqueue(
        &ada.machine,
        person,
        &[CHANNEL, &channel, TO, &unlisted, FOR_NOBODY],
    );

    assert_refused_naming(&none, &[UNLISTED], "an addressee no value lists");

    let two = enqueue(
        &ada.machine,
        person,
        &[CHANNEL, &channel, TO, &twice, FOR_TWO],
    );

    assert_refused_naming(&two, &[ADA_HOST, LAPTOP], "an addressee two devices list");
    assert_none_uploaded(cloud, since, "a task to no or two devices");
    let held = tasks(cloud, ada, at, queue);
    assert!(held.is_empty(), "a task stored: {held:?}");
}
