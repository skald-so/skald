//! A task the server refuses for now, with a rate limit or its failure, waits in the local state,
//! and the device's next run uploads it again with the same bytes, until the server stores it.

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::CHANNEL;
use crate::queuing::{
    Sending, enqueue, occurrences, refers_to_task, sending, task_uploads, tasks, the_one_task,
    upload_request,
};

/// What claude enqueues; no text is anywhere else.
const LATER: &str = "retry test: claude asks for the weekly report while the server is busy";

/// A problem details object of `status`, its problem `word`.
fn problem(status: u16, word: &str) -> String {
    format!(
        r#"{{"type":"https://skald.so/problems/{word}","title":"{word}","status":{status},"detail":"retry test"}}"#
    )
}

/// The relay answers each upload of a task to the channel's `default` with `429 Too Many
/// Requests`: as claude in `~/work/acme`, `skald enqueue --channel <channel> <text>` fails, and
/// the server holds no task. With `503 Service Unavailable` instead, claude's next `skald recv`
/// uploads it again, and the server still holds none. Once uploads reach the server, claude's next
/// `skald recv` uploads it a third time: the server holds that one task, the first run named its
/// `#<ref>`, and every upload carried the bytes the server holds.
#[test]
fn task_the_server_refuses_for_now_waits_and_the_next_run_uploads_the_same_bytes() {
    let agents = Agents::new(
        [4_265_071_001, 4_265_071_002],
        ["ada-retries-her-task", "bob-sees-one-retried-task"],
    );
    agents.run_as(CLAUDE);
    let ada_s: Sending = sending(&agents.talk);
    let (at, queue) = (ada_s.at, ada_s.queue);
    let upload = upload_request(at, queue);
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let since = cloud.relay.mark();
    let claude = (agents.folder.as_path(), Some(CLAUDE));
    let channel = ada_s.channel;
    cloud.relay.refuse_with(
        &upload,
        "429 Too Many Requests",
        &problem(429, "rate-limited"),
    );

    let first = enqueue(&ada.machine, claude, &[CHANNEL, &channel, LATER]);

    assert!(!first.success, "enqueued past a 429: {first:?}");
    assert!(tasks(cloud, ada, at, queue).is_empty(), "a task stored");
    cloud.relay.stop_refusing();
    cloud.relay.refuse_with(
        &upload,
        "503 Service Unavailable",
        &problem(503, "unavailable"),
    );
    agents.run_as(CLAUDE);
    assert!(tasks(cloud, ada, at, queue).is_empty(), "a task stored");
    cloud.relay.stop_refusing();

    agents.run_as(CLAUDE);

    let stored = the_one_task(cloud, ada, at, queue);
    assert!(
        refers_to_task(&first.shown(), stored.task.task),
        "the task's #<ref> not named: {first:?}"
    );
    let uploads = task_uploads(cloud, since);
    assert_eq!(uploads.len(), 3, "not three uploads: {uploads:?}");
    assert_eq!(
        occurrences(&cloud.relay.sent_bytes(), &stored.bytes),
        3,
        "an upload with other bytes than the server holds: {uploads:?}"
    );
}
