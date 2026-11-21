//! Cases beyond the acceptance tests: a revoked uploader's task matches no claim; a repeat of the
//! holder's ending act from another device is `claim-lost`.

use skald_proto::Claim;

use crate::common::Response;
use crate::holder::{
    ALL, DEAD, DONE, FAILED, Holder, assert_claim_lost, assert_taken, scene, takes,
};
use crate::kit::{Client, QUEUED};
use crate::scene::revoke;

/// alice's laptop enqueues a task and is then revoked: claims of every level by bob's and carol's
/// laptops get none, and reads show the task `queued` with attempts 0.
#[test]
fn a_revoked_uploaders_task_goes_to_no_claim() {
    let (w, [_, bob, carol, ..], default, task) = scene(3_927_000_001);
    revoke(&w.fixture().team.alice);

    for client in [&bob, &carol] {
        client.gets_none(default, &takes(&ALL));
    }
    let read = carol.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the claims");
}

/// An ending act on `claim` by `client`.
type Ending = fn(&Client, &Claim) -> Response;

/// Once carol's laptop completes a task, fails one without retry and releases one, the same act on
/// that claim from bob's laptop and from alice's laptop gets `claim-lost`, and reads show each
/// task as carol's act left it.
#[test]
fn another_devices_repeat_of_the_holders_ending_act_is_claim_lost() {
    let (w, [alice, bob, carol, ..], default, _) = scene(3_927_000_011);
    let endings: [(&str, Ending, _); 3] = [
        (
            "complete",
            |c, claim| c.complete(claim),
            (DONE, 1, None, None),
        ),
        (
            "fail",
            |c, claim| c.fail(claim, false),
            (DEAD, 1, None, Some(FAILED)),
        ),
        (
            "release",
            |c, claim| c.release(claim),
            (QUEUED, 0, None, None),
        ),
    ];

    for (act, ending, shows) in endings {
        alice.stores(&w.task(default));
        let claim = carol.claims(default, &takes(&ALL));
        assert_taken(&ending(&carol, &claim), &format!("carol's {act}"));
        for client in [&bob, &alice] {
            let what = format!("{}'s {act}", client.who);
            assert_claim_lost(&ending(client, &claim), &what);
        }
        let read = alice.state_of(default, claim.task.task);
        assert_eq!(read.shows(), shows, "after the repeats of the {act}");
    }
}
