//! What a claim gets: the first task in storing order that is `queued` and matches it, by its
//! addressee's device, its uploader's trust level for the claimer and its template; or none.

use serde_json::json;
use skald_proto::{Cbor, QueueMode, QueuedTask};

use crate::holder::{
    ALL, CLAIMED, Holder, Last, OWN, PEER, SERVICE, assert_claim, scene, takes, takes_templates,
};
use crate::kit::{AT_MOST_ONCE, DEFAULT, LEASE, UNKNOWN_QUEUE, World, assert_problem, named};
use crate::tokens::{QUEUES, SEND};

/// `task` naming the template `name`.
fn with_template(mut task: QueuedTask, name: &str) -> QueuedTask {
    task.sched.template = Some(name.parse().expect("a template name"));
    task
}

/// alice's laptop enqueues three tasks; carol's laptop's claims get them in that order, each of
/// generation 1 and attempt 1, with `default`'s mode and lease and alice's laptop as the uploader;
/// a fourth claim gets none.
#[test]
fn a_claim_gets_the_first_queued_task_in_storing_order() {
    let w = World::new(3_926_100_001);
    let [alice, _, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let tasks: Vec<QueuedTask> = (0..3).map(|_| w.task(default)).collect();
    for task in &tasks {
        alice.stores(task);
    }

    for (n, task) in tasks.iter().enumerate() {
        let what = format!("claim {}", n + 1);
        let claim = carol.claims(default, &takes(&ALL));
        assert_claim(&claim, task, 1, 1, &what);
        assert_eq!(
            (claim.lease, claim.mode, claim.uploader),
            (LEASE, QueueMode::AtLeastOnce, w.alice.id),
            "{what}: the lease, mode and uploader"
        );
    }
    carol.gets_none(default, &takes(&ALL));
}

/// A claim of a task of alice's `at-most-once` queue with a lease of 45 seconds carries that mode
/// and lease.
#[test]
fn a_claim_carries_its_queues_mode_and_lease() {
    let w = World::new(3_926_100_011);
    let [alice, _, carol, ..] = w.laptops();
    let deploys = alice
        .creates(&named(
            "deploys",
            json!({ "mode": AT_MOST_ONCE, "lease": 45 }),
        ))
        .id;
    let task = w.task(deploys);
    alice.stores(&task);

    let claim = carol.claims(deploys, &takes(&ALL));
    assert_claim(&claim, &task, 1, 1, "the claim");
    assert_eq!((claim.lease, claim.mode), (45, QueueMode::AtMostOnce));
}

/// Once carol's laptop claims alice's task, reads show it `claimed` with attempts 1, no progress
/// and the claim, generation 1, held by carol's laptop.
#[test]
fn a_claimed_task_shows_in_reads_with_its_claim_and_no_progress() {
    let (w, [_, bob, carol, ..], default, task) = scene(3_926_100_021);

    let claim = carol.claims(default, &takes(&ALL));
    let read = bob.state_of(default, task.task);
    assert_eq!(read.shows(), (CLAIMED, 1, None, None));
    assert_eq!(read.claim, Some(Last::of(&claim, w.carol.id)));
    assert_eq!(read.bytes, task.to_cbor(), "the bytes read");
}

/// The bot, bob's laptop and alice's laptop enqueue a task each, in that order. carol's laptop,
/// stating `own`, gets none of them. alice's laptop, stating `own`, gets her own task and then
/// none; stating `peer`, bob's and then none; stating `service`, the bot's.
#[test]
fn a_claim_gets_only_tasks_of_the_trust_levels_it_states() {
    let w = World::new(3_926_100_031);
    let [alice, bob, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let bot = w.bot(&[SEND, QUEUES]);
    let [bots, bobs, alices] = [&bot, &bob, &alice].map(|client| {
        let task = w.task(default);
        client.stores(&task);
        task
    });

    carol.gets_none(default, &takes(&[OWN]));
    let own = alice.claims(default, &takes(&[OWN]));
    assert_claim(&own, &alices, 1, 1, "stating own");
    alice.gets_none(default, &takes(&[OWN]));
    let peer = alice.claims(default, &takes(&[PEER]));
    assert_claim(&peer, &bobs, 1, 1, "stating peer");
    alice.gets_none(default, &takes(&[PEER]));
    let service = alice.claims(default, &takes(&[SERVICE]));
    assert_claim(&service, &bots, 1, 1, "stating service");
    assert_eq!(service.uploader, w.stage.bot.id, "the bot uploaded it");
}

/// alice's laptop enqueues a task with carol's laptop in `sched`, then one with no addressee. bob's
/// laptop gets the second; alice's laptop gets none; carol's laptop gets the first.
#[test]
fn a_task_with_an_addressee_goes_only_to_its_device() {
    let w = World::new(3_926_100_041);
    let [alice, bob, carol, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let mut addressed = w.task(default);
    addressed.sched.device = Some(w.carol.id);
    let open = w.task(default);
    alice.stores(&addressed);
    alice.stores(&open);

    let bobs = bob.claims(default, &takes(&ALL));
    assert_claim(&bobs, &open, 1, 1, "bob's claim");
    alice.gets_none(default, &takes(&ALL));
    let carols = carol.claims(default, &takes(&ALL));
    assert_claim(&carols, &addressed, 1, 1, "carol's claim");
}

/// alice's laptop enqueues a task naming the template `review`, then one naming none. bob's laptop
/// gets neither stating no template without tasks without one, nor stating `test` alone; stating
/// `test` and taking tasks without one, it gets the second; stating `review` alone, the first.
#[test]
fn a_task_naming_a_template_goes_only_to_a_claim_stating_it() {
    let w = World::new(3_926_100_051);
    let [alice, bob, ..] = w.laptops();
    let default = alice.queue(DEFAULT).id;
    let review = with_template(w.task(default), "review");
    let plain = w.task(default);
    alice.stores(&review);
    alice.stores(&plain);

    bob.gets_none(default, &takes_templates(&ALL, &[], false));
    bob.gets_none(default, &takes_templates(&ALL, &["test"], false));
    let untemplated = bob.claims(default, &takes_templates(&ALL, &["test"], true));
    assert_claim(
        &untemplated,
        &plain,
        1,
        1,
        "taking tasks without a template",
    );
    let templated = bob.claims(default, &takes_templates(&ALL, &["review"], false));
    assert_claim(&templated, &review, 1, 1, "stating review");
}

/// A claim of a queue the channel does not have gets `unknown-queue`.
#[test]
fn a_claim_of_a_queue_the_channel_does_not_have_is_refused() {
    let w = World::new(3_926_100_061);
    let [alice, ..] = w.laptops();
    let response = alice.claim(skald_proto::QueueId::generate(), &takes(&ALL));
    assert_problem(&response, UNKNOWN_QUEUE, "an unknown queue");
}
