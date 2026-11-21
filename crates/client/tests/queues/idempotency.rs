//! A task is stored once per `idempotency_key` and across a crash: a second task with the key is
//! refused, naming the first; a CLI killed after its upload retries with the same bytes.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{CHANNEL, kill_once, strs};
use crate::queuing::{
    DUPLICATE, ENQUEUE, IDEMPOTENCY_KEY_FLAG, Sending, assert_refused_naming,
    assert_says_reference, assert_task, enqueue, enqueued, occurrences, opened, refers_to_task,
    sending, task_uploads, the_one_task, upload_request,
};
use crate::relay::Lose;
use crate::world::{AGENT, now};

/// What the tests enqueue; no text is anywhere else.
const FIRST: &str = "idempotency test: claude asks for the nightly build of the api";
const SECOND: &str = "idempotency test: claude asks again for the nightly build";
const KILLED: &str = "idempotency test: claude asks for a crash report before it is killed";

/// The key both of claude's tasks state.
const KEY: &str = "nightly-build-api";

/// As claude in `~/work/acme`, `skald enqueue --channel <channel> --idempotency-key
/// nightly-build-api <text>` says its task's `#<ref>`; the server holds it with the key's bytes as
/// its `idempotency_key`. The same command with another text fails, holds the word `duplicate` and
/// the first task's `#<ref>`, and the server still holds the first task alone, as it was; so it
/// does after claude's next `skald recv`.
#[test]
fn second_task_with_the_same_idempotency_key_is_refused_naming_the_first() {
    let agents = Agents::new(
        [4_265_021_001, 4_265_021_002],
        ["ada-keys-the-nightly", "bob-sees-one-nightly"],
    );
    agents.run_as(CLAUDE);
    let Sending {
        at, channel, queue, ..
    } = sending(&agents.talk);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let claude = (agents.folder.as_path(), Some(CLAUDE));

    let first = enqueued(
        &ada.machine,
        claude,
        &[CHANNEL, &channel, IDEMPOTENCY_KEY_FLAG, KEY, FIRST],
    );

    let stored = the_one_task(cloud, ada, at, queue);
    assert_says_reference(&first, stored.task.task);
    assert_eq!(
        stored.task.sched.idempotency_key.as_deref(),
        Some(KEY.as_bytes()),
        "{:?}",
        stored.task.sched
    );

    let second = enqueue(
        &ada.machine,
        claude,
        &[CHANNEL, &channel, IDEMPOTENCY_KEY_FLAG, KEY, SECOND],
    );

    assert_refused_naming(&second, &[DUPLICATE], "the second task with the key");
    assert!(
        refers_to_task(&second.shown(), stored.task.task),
        "the first task's #<ref> not named: {second:?}"
    );
    assert_eq!(
        the_one_task(cloud, ada, at, queue),
        stored,
        "the server's task changed"
    );
    agents.run_as(CLAUDE);
    assert_eq!(
        the_one_task(cloud, ada, at, queue),
        stored,
        "the server's task changed after claude's next run"
    );
}

/// The relay withholds the server's answer to the next upload of a task to the channel's
/// `default`. As claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` uploads its
/// task, and the test kills the run once the server's answer is lost. Claude's next `skald recv`
/// uploads the task again. Every upload carried the bytes the server holds, the server holds that
/// one task, and it opens to the text, from claude.
#[test]
fn cli_killed_after_its_upload_retries_with_the_same_bytes_and_one_task_is_stored() {
    let mut agents = Agents::new(
        [4_265_022_001, 4_265_022_002],
        ["ada-killed-after-enqueue", "bob-sees-one-crash-task"],
    );
    agents.run_as(CLAUDE);
    let address = agents.address(CLAUDE);
    let ada_s = sending(&agents.talk);
    let (at, queue, channel) = (ada_s.at, ada_s.queue, &ada_s.channel);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    cloud
        .relay
        .lose_answer(&upload_request(at, queue), Lose::Withhold);
    let (since, before) = (cloud.relay.mark(), now());
    let args = [AGENT, CLAUDE, ENQUEUE, CHANNEL, channel, KILLED].map(str::to_owned);
    let started = ada.machine.start_in(&agents.folder, &strs(&args));

    kill_once(started, "a lost answer to the upload", || {
        cloud.relay.lost() > 0
    });
    agents.run_as(CLAUDE);

    let after = now();
    let talk = &agents.talk;
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let stored = the_one_task(cloud, ada, at, queue);
    let uploads = task_uploads(cloud, since);
    assert!(
        uploads.len() >= 2,
        "the task not uploaded again: {uploads:?}"
    );
    let sent = cloud.relay.sent_bytes();
    assert_eq!(
        occurrences(&sent, &stored.bytes),
        uploads.len(),
        "an upload with other bytes than the server holds: {uploads:?}"
    );
    let opened = opened(&mut agents.talk, at, (queue, &stored), &ada_s.signing);
    assert_task(
        &opened,
        (at, queue, &stored),
        (ada_s.device, &address, KILLED),
        (before, after),
    );
}
