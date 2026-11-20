//! The rows of "Task states" and "Modes and retries", with attempts, generation, progress and
//! reason as reads show them: release waits and the 5th release, backoffs, `max_attempts`, the end
//! of a lease, fails with and without retry, `at-most-once` tasks and restarts.

use serde_json::json;
use skald_proto::QueueMode;

use crate::holder::{
    ALL, DEAD, DONE, FAILED, Holder, LEASE_EXPIRED, Last, RELEASED, STARTED, assert_claim,
    assert_taken, enqueued, held, passes, scene, takes,
};
use crate::kit::{AT_MOST_ONCE, DEFAULT, LEASE, QUEUED, World, named};

/// carol's laptop claims alice's task and releases it four times: after each, reads show it
/// `queued` with attempts 0, and carol's laptop gets nothing for 10, 20, 40 and 80 seconds, then
/// the task with the next generation and attempt 1. The 5th release makes it `dead` with
/// `released`, and no claim gets it.
#[test]
fn releases_wait_10_20_40_and_80_seconds_and_the_fifth_is_dead_released() {
    let (w, [alice, bob, carol, ..], default, task) = scene(3_926_500_001);

    for (n, wait) in [(1, 10), (2, 20), (3, 40), (4, 80)] {
        let claim = carol.claims(default, &takes(&ALL));
        assert_claim(&claim, &task, n, 1, &format!("claim {n}"));
        assert_taken(&carol.release(&claim), &format!("release {n}"));
        let read = alice.state_of(default, task.task);
        assert_eq!(read.shows(), (QUEUED, 0, None, None), "after release {n}");
        carol.gets_none(default, &takes(&ALL));
        passes(w.db(), task.task, wait - 2);
        carol.gets_none(default, &takes(&ALL));
        passes(w.db(), task.task, 3);
    }
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 5, 1, "claim 5");
    assert_taken(&carol.release(&claim), "release 5");
    let read = alice.state_of(default, task.task);
    assert_eq!(
        read.shows(),
        (DEAD, 0, None, Some(RELEASED)),
        "after release 5"
    );
    assert_eq!(read.claim, Some(Last::of(&claim, w.carol.id)));
    passes(w.db(), task.task, 3_600);
    bob.gets_none(default, &takes(&ALL));
    carol.gets_none(default, &takes(&ALL));
}

/// In a queue of 100 attempts, carol's laptop fails alice's task with retry 11 times: after the
/// n-th, reads show it `queued` with attempts n, and no claim gets it until its backoff of
/// `min(10 × 2^(n−1), 3600)` seconds has passed; then it comes with generation and attempt n + 1.
#[test]
fn backoffs_double_from_10_seconds_up_to_3600() {
    let w = World::new(3_926_500_011);
    let [alice, bob, carol, ..] = w.laptops();
    let retries = alice
        .creates(&named("retries", json!({ "max_attempts": 100 })))
        .id;
    let task = w.task(retries);
    alice.stores(&task);

    let backoffs = [10, 20, 40, 80, 160, 320, 640, 1_280, 2_560, 3_600, 3_600];
    for (n, backoff) in (1..).zip(backoffs) {
        let claim = carol.claims(retries, &takes(&ALL));
        assert_claim(&claim, &task, n, n, &format!("claim {n}"));
        assert_taken(&carol.fail(&claim, true), &format!("fail {n}"));
        let read = alice.state_of(retries, task.task);
        assert_eq!(read.shows(), (QUEUED, n, None, None), "after fail {n}");
        passes(w.db(), task.task, backoff - 2);
        bob.gets_none(retries, &takes(&ALL));
        passes(w.db(), task.task, 3);
    }
    let claim = bob.claims(retries, &takes(&ALL));
    assert_claim(&claim, &task, 12, 12, "after the last backoff");
}

/// carol's laptop claims alice's task and lets its lease end: reads show it `queued` with attempts
/// 1, and no claim gets it until its backoff of 10 seconds has passed; bob's laptop then gets it
/// with generation and attempt 2.
#[test]
fn an_at_least_once_task_whose_lease_ends_waits_its_backoff() {
    let (w, [alice, bob, carol, ..], default, task, claim) = held(3_926_500_021);
    assert_taken(&carol.extend(&claim, STARTED), "carol's extend");

    passes(w.db(), task.task, LEASE + 1);
    let read = alice.awaits(default, task.task, QUEUED);
    assert_eq!(read.shows(), (QUEUED, 1, None, None), "after the lease");
    bob.gets_none(default, &takes(&ALL));
    carol.gets_none(default, &takes(&ALL));
    passes(w.db(), task.task, 10);
    let second = bob.claims(default, &takes(&ALL));
    assert_claim(&second, &task, 2, 2, "after the backoff");
}

/// In a queue of 2 attempts: a task failed with retry twice is `dead` with `failed` and attempts 2;
/// a task whose lease ends twice is `dead` with `lease-expired` and attempts 2. No claim gets
/// either.
#[test]
fn max_attempts_ends_retries_in_the_dlq() {
    let w = World::new(3_926_500_031);
    let [alice, bob, carol, ..] = w.laptops();
    let twice = alice
        .creates(&named("twice", json!({ "max_attempts": 2 })))
        .id;

    let failing = w.task(twice);
    alice.stores(&failing);
    let first = carol.claims(twice, &takes(&ALL));
    assert_taken(&carol.fail(&first, true), "the first fail");
    passes(w.db(), failing.task, 11);
    let second = carol.claims(twice, &takes(&ALL));
    assert_claim(&second, &failing, 2, 2, "the second claim");
    assert_taken(&carol.fail(&second, true), "the second fail");
    let read = alice.state_of(twice, failing.task);
    assert_eq!(
        read.shows(),
        (DEAD, 2, None, Some(FAILED)),
        "after two fails"
    );

    let expiring = w.task(twice);
    alice.stores(&expiring);
    carol.claims(twice, &takes(&ALL));
    passes(w.db(), expiring.task, LEASE + 1);
    alice.awaits(twice, expiring.task, QUEUED);
    passes(w.db(), expiring.task, 11);
    let second = carol.claims(twice, &takes(&ALL));
    assert_claim(&second, &expiring, 2, 2, "the second claim");
    passes(w.db(), expiring.task, LEASE + 1);
    let read = alice.awaits(twice, expiring.task, DEAD);
    assert_eq!(
        read.shows(),
        (DEAD, 2, None, Some(LEASE_EXPIRED)),
        "after two leases"
    );

    for task in [&failing, &expiring] {
        passes(w.db(), task.task, 3_600);
    }
    bob.gets_none(twice, &takes(&ALL));
}

/// A fail without retry makes alice's task `dead` with `failed` and attempts 1, below the queue's
/// 5; a complete after an extend makes another `done` with attempts 1 and its claim.
#[test]
fn a_fail_without_retry_is_dead_failed_and_a_complete_done() {
    let w = World::new(3_926_500_041);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let [failed, done] = enqueued(&w, &alice, default);

    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &failed, 1, 1, "the first claim");
    assert_taken(&carol.fail(&claim, false), "the fail");
    let read = alice.state_of(default, failed.task);
    assert_eq!(read.shows(), (DEAD, 1, None, Some(FAILED)));

    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &done, 1, 1, "the second claim");
    assert_taken(&carol.extend(&claim, STARTED), "the extend");
    assert_taken(&carol.complete(&claim), "the complete");
    let read = alice.state_of(default, done.task);
    assert_eq!(read.shows(), (DONE, 1, None, None));
    assert_eq!(read.claim, Some(Last::of(&claim, w.carol.id)));
}

/// In an `at-most-once` queue: a task failed with retry is `dead` with `failed`; one whose lease
/// ends, `dead` with `lease-expired`; neither is claimed again, with `attempt` 2 or at all. A
/// released task is `queued` again, and bob's next claim has generation 2 and `attempt` 1.
#[test]
fn an_at_most_once_task_is_dead_after_its_first_fail_or_lease_end() {
    let w = World::new(3_926_500_051);
    let [alice, bob, carol, ..] = w.laptops();
    let deploys = alice
        .creates(&named("deploys", json!({ "mode": AT_MOST_ONCE })))
        .id;

    let failed = w.task(deploys);
    alice.stores(&failed);
    let claim = carol.claims(deploys, &takes(&ALL));
    assert_eq!(claim.mode, QueueMode::AtMostOnce, "the claim's mode");
    assert_taken(&carol.fail(&claim, true), "the fail with retry");
    let read = alice.state_of(deploys, failed.task);
    assert_eq!(read.shows(), (DEAD, 1, None, Some(FAILED)));

    let expired = w.task(deploys);
    alice.stores(&expired);
    let claim = carol.claims(deploys, &takes(&ALL));
    assert_claim(&claim, &expired, 1, 1, "the claim to expire");
    passes(w.db(), expired.task, LEASE + 1);
    let read = alice.awaits(deploys, expired.task, DEAD);
    assert_eq!(read.shows(), (DEAD, 1, None, Some(LEASE_EXPIRED)));

    for task in [&failed, &expired] {
        passes(w.db(), task.task, 3_600);
    }
    bob.gets_none(deploys, &takes(&ALL));
    carol.gets_none(deploys, &takes(&ALL));

    let released = w.task(deploys);
    alice.stores(&released);
    let claim = carol.claims(deploys, &takes(&ALL));
    assert_taken(&carol.release(&claim), "the release");
    let read = alice.state_of(deploys, released.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the release");
    let next = bob.claims(deploys, &takes(&ALL));
    assert_claim(&next, &released, 2, 1, "bob's claim");
}

/// carol's laptop releases alice's task 5 times: `dead` with `released`. alice's restart makes it
/// `queued` with attempts 0 and no reason; the next claim has generation 6 and attempt 1, and its
/// release, the first since the restart, keeps the task from carol for 10 seconds alone.
#[test]
fn a_restart_makes_a_dead_task_queued_with_attempts_and_releases_0() {
    let (w, [alice, _, carol, ..], default, task) = scene(3_926_500_061);
    for n in 1..=5 {
        let claim = carol.claims(default, &takes(&ALL));
        assert_taken(&carol.release(&claim), &format!("release {n}"));
        passes(w.db(), task.task, 200);
    }
    let read = alice.state_of(default, task.task);
    assert_eq!(
        read.shows(),
        (DEAD, 0, None, Some(RELEASED)),
        "after 5 releases"
    );

    assert_taken(&alice.restart(default, task.task), "alice's restart");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the restart");
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 6, 1, "the claim after the restart");
    assert_taken(&carol.release(&claim), "the release after the restart");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after that release");
    passes(w.db(), task.task, 11);
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 7, 1, "the claim after the wait");
}
