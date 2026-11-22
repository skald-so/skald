//! With the history retention set, the server deletes by itself a `done`, `dead` or `cancelled`
//! task once the retention has passed since it got there, and keeps `queued` and `claimed` tasks
//! whatever their age.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::QueuedTask;

use crate::common::times::DAY;
use crate::holder::{
    ALL, CLAIMED, DONE, Holder, assert_claim, assert_taken, enqueued, passes, takes,
};
use crate::kit::{DEFAULT, QUEUED, UNKNOWN_TASK, World, assert_problem, named};
use crate::presence::held;

/// The server setting of the history retention, in seconds.
const RETENTION: &str = "--history-retention";
/// The retention the tests set: the cloud's.
const KEPT: u64 = 90 * DAY;
/// How soon the server deletes by itself what the retention deletes.
const DELETION: Duration = Duration::from_secs(30);

/// Checks that no row holds `task`'s `ct` within [`DELETION`].
#[track_caller]
fn assert_deleted(w: &World, task: &QueuedTask, what: &str) {
    let deadline = Instant::now() + DELETION;
    while held(w.db(), &task.ct) {
        assert!(
            Instant::now() < deadline,
            "{what}: still kept after {DELETION:?}"
        );
        thread::sleep(Duration::from_millis(250));
    }
}

/// With a retention of 90 days, alice's laptop enqueues five tasks: carol's laptop completes the
/// first, fails the second without retry, alice's laptop cancels the third. 90 days and a second
/// pass for all five; carol's laptop then claims the fourth, and the fifth waits. A task of another
/// queue, completed now, is kept. The first three are deleted: no row holds them and a read gets
/// `unknown-task`; `default` lists the fourth `claimed` and the fifth `queued`.
#[test]
fn the_retention_deletes_finished_tasks_and_keeps_queued_and_claimed_ones() {
    let kept = KEPT.to_string();
    let w = World::with(3_926_800_001, &[RETENTION, &kept]);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let [done, dead, cancelled, claimed, queued] = enqueued(&w, &alice, default);
    let claim = carol.claims(default, &takes(&ALL));
    assert_taken(&carol.complete(&claim), "the complete");
    let claim = carol.claims(default, &takes(&ALL));
    assert_taken(&carol.fail(&claim, false), "the fail");
    assert_taken(&alice.cancel(default, cancelled.task), "the cancel");
    for task in [&done, &dead, &cancelled, &claimed, &queued] {
        passes(w.db(), task.task, KEPT + 1);
    }
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &claimed, 1, 1, "the old task's claim");
    let later = alice.creates(&named("later", json!({}))).id;
    let recent = w.task(later);
    alice.stores(&recent);
    let claim = carol.claims(later, &takes(&ALL));
    assert_taken(&carol.complete(&claim), "the recent complete");

    for (task, what) in [(&done, "done"), (&dead, "dead"), (&cancelled, "cancelled")] {
        assert_deleted(&w, task, what);
        let response = alice.read(default, task.task, true);
        assert_problem(&response, UNKNOWN_TASK, what);
    }
    let mut listed: Vec<_> = (alice.states(default).into_iter())
        .map(|read| (read.task().to_string(), read.state))
        .collect();
    listed.sort();
    let mut expected = vec![
        (claimed.task.to_string(), CLAIMED.to_owned()),
        (queued.task.to_string(), QUEUED.to_owned()),
    ];
    expected.sort();
    assert_eq!(listed, expected, "the tasks kept in default");
    let read = alice.state_of(later, recent.task);
    assert_eq!(read.state, DONE, "the task completed now");
}
