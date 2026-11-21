//! Under an epoch the device judged unclean nothing is enqueued, saying why: no task is sealed to
//! a key that a device the members did not admit holds (`docs/protocol/membership.md`, "Clean
//! epochs").

use skald_proto::{Role, Roster};

use crate::agents::{Agents, CLAUDE};
use crate::chain::{bob_commits_empty, carol};
use crate::entries::killed_in_unclean_epoch;
use crate::fixtures::CHANNEL;
use crate::kit::rosters;
use crate::queuing::{
    EPOCH, Sending, assert_none_uploaded, assert_refused_naming, assert_says_reference,
    assert_task, enqueue, enqueued, opened, sending, tasks, the_one_task,
};
use crate::talk::Talk;
use crate::world::{ACME, now, serve_revisions};

/// What claude enqueues in each epoch; no text is anywhere else.
const IN_UNCLEAN: &str = "unclean epoch test: claude asks for the audit log export";
const IN_CLEAN: &str = "unclean epoch test: claude asks for the audit log once more";

/// As the test of history entries in an unclean epoch (`history_entries/unclean.rs`): the relay
/// keeps the later revisions of `acme` from Ada's machine; Bob's device, an admin's, adds Carol to
/// the talk's channel, and her device to its group, epoch 2; Ada's run is killed once it entered
/// epoch 2 and judged it unclean, before it commits Carol's Remove; then the revisions go through.
/// As claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` fails, holds the word
/// `epoch`, uploads nothing, and the channel's `default` holds no task. Bob's device then commits
/// an empty Commit, epoch 3, which Ada's machine judges clean: claude's next `skald enqueue` puts
/// a task there under epoch 3, which Bob's key of epoch 3 opens.
#[test]
fn nothing_is_enqueued_under_an_epoch_judged_unclean() {
    let talk = Talk::with(
        ACME,
        &[4_265_031_001, 4_265_031_002],
        &["ada-enqueues-in-the-dark", "bob-adds-carol-unseen"],
        false,
        (Role::Admin, |_| {}),
    );
    let mut agents = Agents::bound(talk);
    agents.run_as(CLAUDE);
    hide_later_revisions(&agents.talk);
    let admitted = carol(&agents.talk, 4_265_031_003, "carol-admitted-unseen");
    killed_in_unclean_epoch(&mut agents.talk, &admitted);
    let claude = agents.address(CLAUDE);
    let Sending {
        at,
        channel,
        queue,
        device: ada_device,
        signing,
    } = sending(&agents.talk);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let as_claude = (agents.folder.as_path(), Some(CLAUDE));
    let since = cloud.relay.mark();

    let refused = enqueue(&ada.machine, as_claude, &[CHANNEL, &channel, IN_UNCLEAN]);

    assert_refused_naming(&refused, &[EPOCH], "a task in epoch 2");
    assert_none_uploaded(cloud, since, "a task in epoch 2");
    let held = tasks(cloud, ada, at, queue);
    assert!(held.is_empty(), "a task in epoch 2: {held:?}");

    bob_commits_empty(&mut agents.talk, 3);
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let before = now();

    let ran = enqueued(&ada.machine, as_claude, &[CHANNEL, &channel, IN_CLEAN]);

    let after = now();
    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&ran, stored.task.task);
    assert_eq!(stored.task.epoch, 3, "not sealed under epoch 3: {stored:?}");
    let opened = opened(&mut agents.talk, at, (queue, &stored), &signing);
    assert_task(
        &opened,
        (at, queue, &stored),
        (ada_device, &claude, IN_CLEAN),
        (before, after),
    );
}

/// The relay serves Ada's machine the revisions of `talk`'s space it has now, and no later one.
fn hide_later_revisions(talk: &Talk) {
    let (cloud, space) = (talk.cloud(), talk.space.id);
    let now_held = rosters(cloud, &talk.ada.token, space);
    serve_revisions(cloud, space, &now_held.iter().collect::<Vec<&Roster>>());
}
