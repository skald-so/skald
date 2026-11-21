//! Lease ends and deadlines apply as they pass, on any instance: reads on another instance than the
//! one that took the claims and tasks show them, with no request to the task.

use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use skald_proto::{QueueId, TaskId};

use crate::holder::{
    ALL, DEAD, DEADLINE_PASSED, Holder, LEASE_EXPIRED, READS_WITHIN, Read, passes, takes,
};
use crate::kit::{AT_MOST_ONCE, Client, DEFAULT, LEASE, QUEUED, World, named};
use crate::wire::now;

/// Reads `queue`'s list through `client` until `task` shows `expected`, its state, attempts,
/// progress and reason, within `within`.
#[track_caller]
fn listed_as(
    client: &Client,
    queue: QueueId,
    task: TaskId,
    expected: (&str, u64, Option<&str>, Option<&str>),
    within: Duration,
) {
    let deadline = Instant::now() + within;
    loop {
        let reads = client.states(queue);
        let found: Option<Read> = reads.into_iter().find(|read| read.task() == task);
        let found = found.unwrap_or_else(|| panic!("{task} not listed"));
        if found.shows() == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{task} still {:?} after {within:?}, not {expected:?}",
            found.shows()
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// On the first instance, alice's laptop claims a task of an `at-most-once` queue and one of
/// `default`, and enqueues a task with a `deadline` 2 seconds ahead. Once the leases have passed
/// and the deadline too, carol's laptop on a second instance lists them: `dead` with
/// `lease-expired`, `queued` with attempts 1, and `dead` with `deadline-passed`.
#[test]
fn a_lease_end_and_a_deadline_show_in_reads_on_the_other_instance() {
    let w = World::new(3_926_700_001);
    let other = w.fixture().team.flow.stand.instance(&[], &[]);
    let [alice, ..] = w.laptops();
    let carol = w.laptop_on(&other, &w.fixture().team.carol);
    let default = alice.queue(DEFAULT).id;
    let deploys = alice
        .creates(&named("deploys", json!({ "mode": AT_MOST_ONCE })))
        .id;
    let once = w.task(deploys);
    let retried = w.task(default);
    for task in [&once, &retried] {
        alice.stores(task);
        alice.claims(task.queue, &takes(&ALL));
    }
    let deadline = now() + 2;
    let mut late = w.task(default);
    late.sched.deadline = Some(deadline);
    alice.stores(&late);

    for task in [&once, &retried] {
        passes(w.db(), task.task, LEASE + 1);
    }
    let within = READS_WITHIN + Duration::from_secs(3);
    listed_as(
        &carol,
        deploys,
        once.task,
        (DEAD, 1, None, Some(LEASE_EXPIRED)),
        within,
    );
    listed_as(
        &carol,
        default,
        retried.task,
        (QUEUED, 1, None, None),
        within,
    );
    listed_as(
        &carol,
        default,
        late.task,
        (DEAD, 0, None, Some(DEADLINE_PASSED)),
        within,
    );
}
