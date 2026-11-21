//! A `service` device outside the group enqueues to the queue key object of the channel's latest
//! GroupInfo and epoch, and only when that GroupInfo passes its checks, lists the object's `signer`
//! as a member and the object's signature verifies with the certificate of `signer`. Its runs leave
//! nothing on its machine's disk.

use skald_proto::{ChannelId, SpaceId};

use crate::fixtures::CHANNEL;
use crate::kit::group_info_request;
use crate::machine::Finished;
use crate::objects::{object_of, pk_of};
use crate::queuing::{
    ENQUEUE, QUEUES, assert_none_uploaded, assert_refused_naming, assert_task, assert_week_after,
    default_queue, opened, tasks, the_one_task,
};
use crate::scene::{Scene, change_field, other_device};
use crate::service_kit::{PERMISSION, RECEIVE, SEND};
use crate::world::now;

/// What the agent enqueues; nowhere else.
const DEPLOY: &str = "stateless queue test: deploy-bot asks for a smoke test of the release";

/// The problem the server names for a token without `queues`.
const NOT_A_MEMBER: &str = "not-a-member";

/// The options that give the agent's token `send`, `receive` and `queues`.
const ALL_PERMISSIONS: [&str; 6] = [PERMISSION, SEND, PERMISSION, RECEIVE, PERMISSION, QUEUES];

/// The scene, its agent's token with `send`, `receive` and `queues`; Bob's device took in its
/// mailbox.
fn queuing_scene(ids: [u64; 2], logins: [&str; 2]) -> Scene {
    let mut scene = Scene::with(ids, logins, &ALL_PERMISSIONS);
    scene.talk.played[0].take_in(&scene.talk.world.cloud);
    scene
}

/// The request for any queue key object of `at`'s channel, as its line starts.
fn objects_request((space, channel): (SpaceId, ChannelId)) -> String {
    format!("GET /spaces/{space}/channels/{channel}/queue-keys/")
}

/// The agent's `skald enqueue --channel <channel> <DEPLOY>`, leaving nothing on its machine's disk.
#[track_caller]
fn enqueue(scene: &Scene) -> Finished {
    let channel = scene.channel().to_string();
    scene.agent.run(&[ENQUEUE, CHANNEL, &channel, DEPLOY])
}

/// Runs the agent's enqueue and fails unless it fails, saying why, uploads no task, and the server
/// holds none; `what` names the case.
#[track_caller]
fn refused(scene: &Scene, what: &str) -> Finished {
    let (cloud, at) = (scene.cloud(), scene.talk.at());
    let since = cloud.relay.mark();
    let ran = enqueue(scene);
    assert!(!ran.success, "{what}: the enqueue succeeded: {ran:?}");
    assert!(
        !ran.stderr.trim().is_empty(),
        "{what}: says nothing: {ran:?}"
    );
    assert_none_uploaded(cloud, since, what);
    let queue = default_queue(cloud, scene.ada(), at);
    let held = tasks(cloud, scene.ada(), at, queue);
    assert!(held.is_empty(), "{what}: the server holds tasks: {held:?}");
    ran
}

/// From now on, the relay gives the queue key objects of the scene's channel as `object`.
fn give_object(scene: &Scene, object: Vec<u8>) {
    let request = objects_request(scene.talk.at());
    change_field(scene.cloud(), &request, "queue_key", move |bytes| {
        bytes.clone_from(&object);
    });
}

/// The agent's `skald enqueue --channel <channel> <text>` puts one task in the channel's queue
/// `default`. Bob's device opens it with its history key of the task's epoch, the epoch of the
/// channel's latest GroupInfo; it is signed by the agent's device, its content from the agent's
/// address, with a grant that passes Bob's device's checks and no counter, its `deadline` a week
/// after its time of enqueue, as Bob's account may claim it. The run leaves nothing on the agent's
/// machine's disk.
#[test]
fn a_stateless_agent_enqueues_a_task_the_member_devices_open() {
    let mut scene = queuing_scene(
        [5_419_300_001, 5_419_300_002],
        ["ada-stateless-enqueue", "bob-stateless-enqueue"],
    );
    let (at, signing) = (scene.talk.at(), scene.cert.body().signing);
    let before = now();

    let ran = enqueue(&scene);

    let after = now();
    assert!(ran.success, "the agent's enqueue failed: {ran:?}");
    let (cloud, ada) = (scene.cloud(), scene.ada());
    let queue = default_queue(cloud, ada, at);
    let stored = the_one_task(cloud, ada, at, queue);
    let epoch = scene.talk.played[0].mls.epoch(at.1).expect("Bob's epoch");
    assert_eq!(stored.task.epoch, epoch, "the task's epoch");
    let (device, address) = (scene.device, scene.address.clone());
    let opened = opened(&mut scene.talk, at, (queue, &stored), &signing);
    let task = (at, queue, &stored);
    assert_task(
        &opened,
        task,
        (device, address.as_str(), DEPLOY),
        (before, after),
    );
    scene.assert_granted(opened.content());
    assert_week_after(
        stored.task.sched.deadline,
        opened.body.enqueued,
        (before, after),
    );
}

/// The relay gives the agent the channel's queue key object signed by a valid device of Ada's that
/// the GroupInfo does not list; then the object with a byte of its signature changed; then an object
/// of the next epoch that Ada's device signed. Each time the agent's enqueue fails, saying why, and
/// uploads no task. Given the object the server holds, the agent enqueues.
#[test]
fn a_stateless_agent_enqueues_nothing_to_an_object_that_fails_its_checks() {
    let scene = queuing_scene(
        [5_419_310_001, 5_419_310_002],
        ["ada-stateless-object", "bob-stateless-object"],
    );
    let at = scene.talk.at();
    let bob = &scene.talk.played[0];
    let epoch = bob.mls.epoch(at.1).expect("Bob's epoch");
    let key = pk_of(bob, at.1, epoch);
    let spare = other_device(scene.cloud(), scene.ada(), "ada-spare-queue-key");
    let ada = (scene.ada().device, &scene.ada().key);

    give_object(
        &scene,
        object_of((at.1, epoch), key, (spare.id, &spare.signing)),
    );
    refused(&scene, "an object whose signer the GroupInfo does not list");
    let mut broken = object_of((at.1, epoch), key, ada);
    *broken.last_mut().expect("an object") ^= 1;
    give_object(&scene, broken);
    refused(&scene, "an object with a broken signature");
    give_object(&scene, object_of((at.1, epoch + 1), key, ada));
    refused(&scene, "an object of another epoch");

    scene
        .cloud()
        .relay
        .rewrite(&objects_request(at), str::to_owned);
    let ran = enqueue(&scene);
    assert!(ran.success, "the enqueue given the held object: {ran:?}");
    let queue = default_queue(scene.cloud(), scene.ada(), at);
    the_one_task(scene.cloud(), scene.ada(), at, queue);
}

/// The relay gives the channel's GroupInfo with a byte of its signature changed: the agent's
/// enqueue fails, saying why, and uploads no task. Given the GroupInfo the server holds, the agent
/// enqueues.
#[test]
fn a_stateless_agent_enqueues_nothing_under_a_group_info_that_fails_its_checks() {
    let scene = queuing_scene(
        [5_419_320_001, 5_419_320_002],
        ["ada-stateless-group-info", "bob-stateless-group-info"],
    );
    let request = group_info_request(scene.space(), scene.channel());
    change_field(scene.cloud(), &request, "group_info", |bytes| {
        *bytes.last_mut().expect("a GroupInfo") ^= 1;
    });

    refused(&scene, "a GroupInfo with a broken signature");

    scene.cloud().relay.rewrite(&request, str::to_owned);
    let ran = enqueue(&scene);
    assert!(ran.success, "the enqueue given the held GroupInfo: {ran:?}");
}

/// The agent's token has the creation's default permissions, without `queues`: its enqueue fails
/// naming `not-a-member`, and the server holds no task.
#[test]
fn a_stateless_agent_whose_token_lacks_queues_is_not_a_member() {
    let scene = Scene::new(
        [5_419_330_001, 5_419_330_002],
        ["ada-stateless-no-queues", "bob-stateless-no-queues"],
    );

    let ran = refused(&scene, "a token without queues");

    assert_refused_naming(&ran, &[NOT_A_MEMBER], "a token without queues");
}
