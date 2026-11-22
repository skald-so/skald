//! Tasks no claim gets: one in its backoff, one past its `deadline`, and one its releasing device
//! waits for after its release.

use crate::holder::{
    ALL, DEAD, DEADLINE_PASSED, Holder, Last, assert_claim, assert_taken, held, passes, takes,
    wait_past,
};
use crate::kit::{DEFAULT, QUEUED, World};
use crate::wire::now;

/// carol's laptop fails alice's task with retry: reads show it `queued` with attempts 1. No claim
/// gets it 8 seconds into its backoff of 10; bob's laptop gets it 11 seconds in, of generation 2
/// and attempt 2.
#[test]
fn a_task_in_its_backoff_is_not_handed_out_until_it_ends() {
    let (w, [alice, bob, carol, ..], default, task, first) = held(3_926_200_001);
    assert_taken(&carol.fail(&first, true), "carol's fail");

    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 1, None, None));
    assert_eq!(read.claim, Some(Last::of(&first, w.carol.id)));
    bob.gets_none(default, &takes(&ALL));
    carol.gets_none(default, &takes(&ALL));
    passes(w.db(), task.task, 8);
    bob.gets_none(default, &takes(&ALL));
    passes(w.db(), task.task, 3);
    let second = bob.claims(default, &takes(&ALL));
    assert_claim(&second, &task, 2, 2, "after the backoff");
}

/// alice's laptop enqueues a task with a `deadline` 2 seconds ahead, then one with a `deadline` an
/// hour ahead. Once the first has passed, bob's laptop gets the second and then none, and reads
/// show the first `dead` with `deadline-passed`, attempts 0 and no claim.
#[test]
fn a_task_past_its_deadline_is_not_handed_out_and_goes_dead() {
    let w = World::new(3_926_200_011);
    let [alice, bob, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let deadline = now() + 2;
    let mut late = w.task(default);
    late.sched.deadline = Some(deadline);
    let mut later = w.task(default);
    later.sched.deadline = Some(now() + 3_600);
    alice.stores(&late);
    alice.stores(&later);

    wait_past(deadline + 1);
    let claim = bob.claims(default, &takes(&ALL));
    assert_claim(&claim, &later, 1, 1, "the task before its deadline");
    bob.gets_none(default, &takes(&ALL));
    let read = alice.awaits(default, late.task, DEAD);
    assert_eq!(read.shows(), (DEAD, 0, None, Some(DEADLINE_PASSED)));
    assert_eq!(read.claim, None, "never claimed");
}

/// carol's laptop releases alice's task: reads show it `queued` with attempts 0. carol's laptop
/// gets nothing, as it waits; bob's laptop gets the task at once, of generation 2 and attempt 1.
#[test]
fn a_released_task_is_not_handed_to_its_releasing_device_during_its_wait() {
    let (_w, [alice, bob, carol, ..], default, task, first) = held(3_926_200_021);
    assert_taken(&carol.release(&first), "carol's release");

    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None));
    carol.gets_none(default, &takes(&ALL));
    let second = bob.claims(default, &takes(&ALL));
    assert_claim(&second, &task, 2, 1, "bob's claim after the release");
}
