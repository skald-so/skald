//! Fencing: the server takes an extend, complete, fail or release only from the device holding the
//! task's current claim, with its `claim_id` and `generation`, within the lease, while the task is
//! `claimed`; otherwise `claim-lost`, which changes nothing. A repeated complete, fail or release
//! gets the first answer. A release from a `service` device gets `not-allowed`. An extend renews
//! the lease and sets the progress reads show.

use serde_json::json;
use skald_proto::Claim;

use crate::holder::{
    ALL, AWAITING_APPROVAL, CLAIMED, DEAD, DONE, FAILED, Holder, Last, STARTED, admit_bot,
    assert_claim, assert_claim_lost, assert_same_answer, assert_taken, held, next_generation,
    other_claim_id, passes, scene_after, takes,
};
use crate::kit::{Client, LEASE, NOT_ALLOWED, QUEUED, assert_problem, named};
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// Each of extend, complete, fail with and without retry, and release by `client` on `claim`, with
/// what it is.
fn every_act(client: &Client, claim: &Claim) -> Vec<(crate::common::Response, &'static str)> {
    vec![
        (client.extend(claim, AWAITING_APPROVAL), "an extend"),
        (client.complete(claim), "a complete"),
        (client.fail(claim, true), "a fail with retry"),
        (client.fail(claim, false), "a fail without retry"),
        (client.release(claim), "a release"),
    ]
}

/// carol's laptop claims alice's task and extends it with `started`. Every act with another
/// `claim_id`, and with the next generation, gets `claim-lost`, and reads show the task as before;
/// carol's complete then makes it `done`.
#[test]
fn an_act_with_another_claim_id_or_generation_is_claim_lost_and_changes_nothing() {
    let (_w, [alice, _, carol, ..], default, task, claim) = held(3_926_400_001);
    assert_taken(&carol.extend(&claim, STARTED), "carol's extend");
    let before = alice.state_of(default, task.task);
    assert_eq!(before.shows(), (CLAIMED, 1, Some(STARTED), None));

    for (wrong, which) in [
        (other_claim_id(&claim), "another claim_id"),
        (next_generation(&claim), "the next generation"),
    ] {
        for (response, what) in every_act(&carol, &wrong) {
            assert_claim_lost(&response, &format!("{what} with {which}"));
        }
        assert_eq!(alice.state_of(default, task.task), before, "after {which}");
    }
    assert_taken(&carol.complete(&claim), "carol's complete");
    assert_eq!(
        alice.state_of(default, task.task).shows(),
        (DONE, 1, None, None)
    );
}

/// carol's laptop claims alice's task; every act of bob's laptop and of alice's laptop with carol's
/// `claim_id` and `generation` gets `claim-lost`, and reads show the task as before; carol's
/// complete then makes it `done`.
#[test]
fn an_act_from_another_device_is_claim_lost_and_changes_nothing() {
    let (_w, [alice, bob, carol, ..], default, task, claim) = held(3_926_400_011);
    let before = alice.state_of(default, task.task);

    for client in [&bob, &alice] {
        for (response, what) in every_act(client, &claim) {
            assert_claim_lost(&response, &format!("{what} of {}", client.who));
        }
        assert_eq!(
            alice.state_of(default, task.task),
            before,
            "after {}",
            client.who
        );
    }
    assert_taken(&carol.complete(&claim), "carol's complete");
    assert_eq!(
        alice.state_of(default, task.task).shows(),
        (DONE, 1, None, None)
    );
}

/// carol's laptop claims alice's task; once its lease of 300 seconds has passed, each of carol's
/// acts gets `claim-lost`. Reads show the task `queued` with attempts 1 and carol's claim, as the
/// lease's end alone makes it.
#[test]
fn an_act_after_the_lease_is_claim_lost() {
    let (w, [alice, _, carol, ..], default, task, claim) = held(3_926_400_021);
    passes(w.db(), task.task, LEASE + 1);

    for (response, what) in every_act(&carol, &claim) {
        assert_claim_lost(&response, &format!("{what} after the lease"));
    }
    let read = alice.awaits(default, task.task, QUEUED);
    assert_eq!(read.shows(), (QUEUED, 1, None, None), "after the lease");
    assert_eq!(read.claim, Some(Last::of(&claim, w.carol.id)));
}

/// Once carol's laptop completes a task, its extend, fail and release get `claim-lost`; once it
/// fails one without retry, its extend, complete and release; once it releases one, its extend,
/// complete and fail. Reads show each task as the first act left it.
#[test]
fn an_act_once_the_task_is_no_longer_claimed_is_claim_lost() {
    let (w, [alice, _, carol, ..], default, done, claim) = held(3_926_400_031);
    assert_taken(&carol.complete(&claim), "the complete");
    for (response, what) in [
        (carol.extend(&claim, STARTED), "an extend"),
        (carol.fail(&claim, true), "a fail"),
        (carol.release(&claim), "a release"),
    ] {
        assert_claim_lost(&response, &format!("{what} once done"));
    }
    assert_eq!(
        alice.state_of(default, done.task).shows(),
        (DONE, 1, None, None)
    );

    let dead = w.task(default);
    alice.stores(&dead);
    let claim = carol.claims(default, &takes(&ALL));
    assert_taken(&carol.fail(&claim, false), "the fail");
    for (response, what) in [
        (carol.extend(&claim, STARTED), "an extend"),
        (carol.complete(&claim), "a complete"),
        (carol.release(&claim), "a release"),
    ] {
        assert_claim_lost(&response, &format!("{what} once dead"));
    }
    let read = alice.state_of(default, dead.task);
    assert_eq!(read.shows(), (DEAD, 1, None, Some(FAILED)));

    let released = w.task(default);
    alice.stores(&released);
    let claim = carol.claims(default, &takes(&ALL));
    assert_taken(&carol.release(&claim), "the release");
    for (response, what) in [
        (carol.extend(&claim, STARTED), "an extend"),
        (carol.complete(&claim), "a complete"),
        (carol.fail(&claim, true), "a fail"),
    ] {
        assert_claim_lost(&response, &format!("{what} once released"));
    }
    let read = alice.state_of(default, released.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None));
}

/// carol's laptop repeats a complete, at once and after the lease, a fail with retry and a release:
/// each repeat gets the first answer, and reads show the first's effect alone: `done`; `queued`
/// with attempts 1; `queued` with attempts 0. The release counts once: bob's release of the next
/// claim, the second since enqueue, keeps the task from bob for 20 seconds.
#[test]
fn a_repeated_complete_fail_or_release_gets_the_first_answer() {
    let (w, [alice, bob, carol, ..], default, done, claim) = held(3_926_400_041);
    let first = carol.complete(&claim);
    assert_taken(&first, "the complete");
    assert_same_answer(&carol.complete(&claim), &first, "a repeated complete");
    passes(w.db(), done.task, LEASE + 1);
    assert_same_answer(
        &carol.complete(&claim),
        &first,
        "a complete repeated after the lease",
    );
    assert_eq!(
        alice.state_of(default, done.task).shows(),
        (DONE, 1, None, None)
    );

    let failed = w.task(default);
    alice.stores(&failed);
    let claim = carol.claims(default, &takes(&ALL));
    let first = carol.fail(&claim, true);
    assert_taken(&first, "the fail");
    assert_same_answer(&carol.fail(&claim, true), &first, "a repeated fail");
    let read = alice.state_of(default, failed.task);
    assert_eq!(read.shows(), (QUEUED, 1, None, None), "after the fails");

    let releases = alice.creates(&named("releases", json!({}))).id;
    let released = w.task(releases);
    alice.stores(&released);
    let claim = carol.claims(releases, &takes(&ALL));
    assert_claim(&claim, &released, 1, 1, "the claim to release");
    let first = carol.release(&claim);
    assert_taken(&first, "the release");
    assert_same_answer(&carol.release(&claim), &first, "a repeated release");
    let read = alice.state_of(releases, released.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the releases");
    let next = bob.claims(releases, &takes(&ALL));
    assert_claim(&next, &released, 2, 1, "bob's claim");
    assert_taken(&bob.release(&next), "bob's release");
    passes(w.db(), released.task, 18);
    bob.gets_none(releases, &takes(&ALL));
    passes(w.db(), released.task, 3);
    let again = bob.claims(releases, &takes(&ALL));
    assert_claim(&again, &released, 3, 1, "bob's claim after his wait");
}

/// With the bot in the group, its release of its claim of alice's task gets `not-allowed`, and
/// reads show the task still `claimed` by it; its fail with retry is taken.
#[test]
fn a_release_from_a_service_device_is_not_allowed() {
    let (w, [alice, ..], default, task) = scene_after(3_926_400_051, admit_bot);
    let bot = w.bot(&[SEND, RECEIVE, QUEUES]);
    let claim = bot.claims(default, &takes(&ALL));

    assert_problem(&bot.release(&claim), NOT_ALLOWED, "the bot's release");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, None, None), "after the release");
    assert_eq!(read.claim, Some(Last::of(&claim, w.stage.bot.id)));
    assert_taken(&bot.fail(&claim, true), "the bot's fail");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 1, None, None), "after the fail");
}

/// carol's laptop claims alice's task: no progress. It extends with `started`, 200 seconds later
/// with `awaiting-approval`, 200 seconds later with `started` again, each shown in reads; 200
/// seconds later, 600 since the claim, its complete is taken, as each extend renewed the lease of
/// 300 seconds.
#[test]
fn an_extend_renews_the_lease_and_reads_show_its_progress() {
    let (w, [alice, _, carol, ..], default, task, claim) = held(3_926_400_061);
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, None, None), "after the claim");

    assert_taken(&carol.extend(&claim, STARTED), "the first extend");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, Some(STARTED), None));
    for progress in [AWAITING_APPROVAL, STARTED] {
        passes(w.db(), task.task, 200);
        assert_taken(&carol.extend(&claim, progress), progress);
        let read = alice.state_of(default, task.task);
        assert_eq!(read.shows(), (CLAIMED, 1, Some(progress), None));
        assert_eq!(read.claim, Some(Last::of(&claim, w.carol.id)));
    }
    passes(w.db(), task.task, 200);
    assert_taken(&carol.complete(&claim), "the complete");
    assert_eq!(
        alice.state_of(default, task.task).shows(),
        (DONE, 1, None, None)
    );
}
