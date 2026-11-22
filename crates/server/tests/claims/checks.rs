//! The server's claim checks: only a member device of the channel's current epoch, of kind `client`
//! or `service`, not revoked, and, for a `service` device, on a token with `queues`, claims a task
//! or extends, completes, fails or releases its claim; others get `not-a-member`, and nothing is
//! handed out. Each request proves its device.

use serde_json::json;
use skald_proto::DeviceKind;

use crate::holder::{
    ALL, CLAIMED, Holder, Last, STARTED, admit_bot, assert_claim, client_of, device_of, remove_bob,
    scene, scene_after, takes,
};
use crate::kit::{NOT_A_MEMBER, QUEUED, assert_problem};
use crate::presence::{assert_refused_with, assert_unproved};
use crate::scene::revoke;
use crate::tokens::{QUEUES, RECEIVE, SEND};

/// alice's laptop removes bob's laptop from the group and enqueues a task. Refused with
/// `not-a-member`: dave's laptop, outside the channel; bob's, in the channel but outside its
/// current epoch; and the bot, a `service` device outside the group, on a token with `send` and
/// `queues`. eve's laptop, outside the space, is refused. Reads show the task `queued` with
/// attempts 0, and carol's laptop then claims it, of generation 1.
#[test]
fn a_device_outside_the_group_claims_nothing() {
    let (w, [alice, bob, carol, dave, eve], default, task) = scene_after(3_926_300_001, remove_bob);
    let bot = w.bot(&[SEND, RECEIVE, QUEUES]);

    for client in [&dave, &bob, &bot] {
        let response = client.claim(default, &takes(&ALL));
        assert_problem(&response, NOT_A_MEMBER, &client.who);
    }
    assert_refused_with(&eve.claim(default, &takes(&ALL)), None, &eve.who);
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the refusals");
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 1, 1, "carol's claim");
}

/// With the bot in the group, the bot on a token without `queues` gets `not-a-member`; on a token
/// with it, the bot claims alice's task, of generation 1.
#[test]
fn a_member_service_device_claims_only_on_a_token_with_queues() {
    let (w, [alice, ..], default, task) = scene_after(3_926_300_011, admit_bot);

    let without = w.bot(&[SEND, RECEIVE]);
    let refused = without.claim(default, &takes(&ALL));
    assert_problem(&refused, NOT_A_MEMBER, &without.who);
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the refusal");
    let with = w.bot(&[SEND, RECEIVE, QUEUES]);
    let claim = with.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 1, 1, "the bot's claim");
    let read = alice.state_of(default, task.task);
    assert_eq!(read.claim, Some(Last::of(&claim, w.stage.bot.id)));
}

/// A `browser` device of alice's and carol's laptop once revoked are refused a claim; bob's laptop
/// then claims alice's task, of generation 1.
#[test]
fn a_browser_or_a_revoked_device_claims_nothing() {
    let (w, [alice, bob, carol, ..], default, task) = scene(3_926_300_021);
    let team = &w.fixture().team;
    let browser = device_of(&w, &team.alice, DeviceKind::Browser, "Browser");
    let browsers = client_of(&w, w.server(), &team.alice, &browser);
    revoke(&team.carol);

    for client in [&browsers, &carol] {
        assert_refused_with(&client.claim(default, &takes(&ALL)), None, &client.who);
    }
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (QUEUED, 0, None, None), "after the refusals");
    let claim = bob.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 1, 1, "bob's claim");
}

/// bob's laptop claims alice's task; once alice's laptop removes it from the group, its extend,
/// complete, fail and release get `not-a-member`, and reads show the task still `claimed` by it.
#[test]
fn a_holder_outside_the_group_extends_completes_fails_and_releases_nothing() {
    let (mut w, [alice, bob, ..], default, task) = scene(3_926_300_031);
    let claim = bob.claims(default, &takes(&ALL));
    remove_bob(&mut w);

    let refused = [
        (bob.extend(&claim, STARTED), "an extend"),
        (bob.complete(&claim), "a complete"),
        (bob.fail(&claim, true), "a fail"),
        (bob.release(&claim), "a release"),
    ];
    for (response, what) in &refused {
        assert_problem(response, NOT_A_MEMBER, what);
    }
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, None, None), "after the refusals");
    assert_eq!(read.claim, Some(Last::of(&claim, w.bob.id)));
}

/// carol's session without her laptop's proof, and the member bot's token alone, claim nothing:
/// `401` with `auth-required`. carol's laptop then claims alice's task, of generation 1; without
/// the proof, its extend, complete, fail and release get `401` too, and so do alice's restart and
/// cancel; reads show the task still `claimed` with no progress.
#[test]
fn a_session_or_token_without_the_devices_proof_claims_and_changes_nothing() {
    let (w, [alice, _, carol, ..], default, task) = scene_after(3_926_300_041, admit_bot);
    let bot = w.bot(&[SEND, RECEIVE, QUEUES]);

    for client in [&carol, &bot] {
        let response = client.claim_with(default, &takes(&ALL), false);
        assert_unproved(&response, &client.who);
    }
    let claim = carol.claims(default, &takes(&ALL));
    assert_claim(&claim, &task, 1, 1, "carol's proved claim");
    let unproved = [
        carol.act_with(&claim, "extend", &json!({ "progress": STARTED }), false),
        carol.act_with(&claim, "complete", &json!({}), false),
        carol.act_with(&claim, "fail", &json!({ "retry": false }), false),
        carol.act_with(&claim, "release", &json!({}), false),
        alice.on_task(default, task.task, "restart", false),
        alice.on_task(default, task.task, "cancel", false),
    ];
    for response in &unproved {
        assert_unproved(response, "a request without the proof");
    }
    let read = alice.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, None, None), "after the refusals");
}
