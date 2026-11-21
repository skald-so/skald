//! Tasks the device fails without retry, with a warning to its user, and never hands to the agent:
//! one whose `ct`, signature, `sender`, `sched`, `mode` or content a test device or a compromised
//! server altered, one with a `counter`, one past its `deadline`, and an `at-most-once` claim with
//! `attempt` 2 ("Claim checks").

use std::thread;
use std::time::Duration;

use serde_json::json;
use skald_proto::{QueueMode, TaskId};

use crate::claiming::{Alter, Claims, Craft, DEAD, FAILED, QUEUE_FLAG, tampers, warns_of};
use crate::queuing::{AT_MOST_ONCE, creates_queue};
use crate::world::now;

/// What the tests write; no text is anywhere else.
const CT: &str = "refusal test: a task whose ciphertext a proxy changed";
const SIGNATURE: &str = "refusal test: a task whose signature a proxy changed";
const SENDER: &str = "refusal test: a task naming the laptop as its sender";
const SCHED: &str = "refusal test: a task whose record's sched differs from its body's";
const MODE: &str = "refusal test: a task stating at-most-once in an at-least-once queue";
const CONTENT: &str = "refusal test: a task from bob's address signed by ada's device";
const COUNTER: &str = "refusal test: a task with a counter";
const LATE: &str = "refusal test: a task handed out past its deadline";
const TWICE: &str = "refusal test: an at-most-once task handed out a second time";

/// How far ahead of now the late task's deadline is when it is written, in seconds.
const DEADLINE_AHEAD: u64 = 5;

/// Fails unless each of `tasks`, `(task, text)` of the queue `default`, is `dead` with the reason
/// `failed` on the server, `shown` holds none of their texts, and it has a warning line naming
/// each.
#[track_caller]
fn assert_failed_and_warned(claims: &Claims, tasks: &[(TaskId, String)], shown: &str) {
    for (task, text) in tasks {
        assert!(!shown.contains(text), "{text:?} handed to codex: {shown}");
        let read = claims.read(*task);
        assert_eq!(
            read.summary(),
            (DEAD, Some(FAILED), 1),
            "{text:?}: {read:?}"
        );
        assert!(
            warns_of(shown, *task),
            "{text:?}: no warning naming it: {shown}"
        );
    }
}

/// Ada's first device writes seven tasks to the queue `default`, each breaking "Claim checks"
/// otherwise than by a key the device lacks or a match: its `ct` changed after sealing; its
/// signature changed; naming the second machine's device as its `sender`; its record's `sched`
/// holding an `idempotency_key` its body's lacks; its body stating `at-most-once`; its content
/// `from` Bob's person address; and its content with a `counter`. After codex's claims, the server
/// gives each `dead` with the reason `failed`; the runs show none of their texts and a warning
/// naming each.
#[test]
fn tasks_a_test_device_or_proxy_altered_are_failed_without_retry_with_a_warning() {
    let mut claims = Claims::new(
        [4_267_071_001, 4_267_071_002],
        ["ada-writes-altered-tasks", "bob-lends-his-address"],
    );
    let (person, bob_person, queue) = (claims.person(), claims.bob_person(), claims.queue);
    let laptop = claims.laptop().device;
    let mut mode = Craft::new(&person, queue, MODE);
    mode.mode = QueueMode::AtMostOnce;
    let mut counter = Craft::new(&person, queue, COUNTER);
    counter.counter = Some(1);
    let crafts = [
        Craft::new(&person, queue, CT).altered(Alter::Ct),
        Craft::new(&person, queue, SIGNATURE).altered(Alter::Signature),
        Craft::new(&person, queue, SENDER).altered(Alter::Sender(laptop)),
        Craft::new(&person, queue, SCHED).altered(Alter::RecordSched),
        mode,
        Craft::new(&bob_person, queue, CONTENT),
        counter,
    ];
    let tasks: Vec<(TaskId, String)> = (crafts.iter())
        .map(|craft| (claims.ada_writes(craft).task, craft.text.clone()))
        .collect();

    let shown = claims.codex_claims_times(crafts.len(), &[]);

    assert_failed_and_warned(&claims, &tasks, &shown);
}

/// Ada's first device writes a task to `default` whose `deadline` is 5 seconds ahead, and the
/// server's copy then has no deadline; once it passed, codex's claim gets the task. Ada's first
/// device creates `deploys`, `at-most-once`, and writes a task there, whose claims the server then
/// counts as 1 already, so codex's claim with `--queue deploys` gets it with `attempt` 2. The
/// server gives both `dead` with the reason `failed`; the runs show neither text and a warning
/// naming each.
#[test]
fn task_past_its_deadline_and_an_at_most_once_second_attempt_are_failed_without_retry() {
    let mut claims = Claims::new(
        [4_267_072_001, 4_267_072_002],
        ["ada-hands-out-late-tasks", "bob-sees-them-failed"],
    );
    let person = claims.person();
    let deadline = now() + DEADLINE_AHEAD;
    let mut late = Craft::new(&person, claims.queue, LATE);
    late.sched.deadline = Some(deadline);
    let late = claims.ada_writes(&late).task;
    tampers(claims.cloud(), late, "deadline = NULL");
    let settings = json!({ "name": "deploys", "mode": AT_MOST_ONCE });
    let deploys = creates_queue(claims.cloud(), claims.ada(), claims.at, &settings).id;
    let mut twice = Craft::new(&person, deploys, TWICE);
    twice.mode = QueueMode::AtMostOnce;
    let twice = claims.ada_writes(&twice).task;
    tampers(claims.cloud(), twice, "attempts = 1");
    while now() <= deadline {
        thread::sleep(Duration::from_millis(200));
    }

    let mut shown = claims.codex_claims(&[]).shown();
    shown += &claims.codex_claims(&[QUEUE_FLAG, "deploys"]).shown();

    let late_read = claims.read(late);
    assert_eq!(
        late_read.summary(),
        (DEAD, Some(FAILED), 1),
        "{late_read:?}"
    );
    let twice_read = claims.read_in(deploys, twice);
    assert_eq!(
        twice_read.summary(),
        (DEAD, Some(FAILED), 2),
        "{twice_read:?}"
    );
    for (task, text) in [(late, LATE), (twice, TWICE)] {
        assert!(!shown.contains(text), "{text:?} handed to codex: {shown}");
        assert!(
            warns_of(&shown, task),
            "{text:?}: no warning naming it: {shown}"
        );
    }
}
