//! Member devices upload the queue key object of each clean epoch: the sender of a Commit, of the
//! epoch it starts; any member device after its catch-up, of the current epoch if the server has
//! none; once when a proxy cut the first upload; none of an unclean epoch. A member agent uploads as
//! a member's device does.

use crate::chain::{adds_to_channel, bob_commits_empty, carol, commits_empty};
use crate::fixtures::cert_of;
use crate::joins::{intruder, intruder_joins};
use crate::kit::{RECV, received};
use crate::member::Bot;
use crate::objects::{assert_object, held, held_one, pk_of, uploads_of};
use crate::service_kit::service_of;
use crate::talk::Talk;
use crate::world::run;

/// Fails unless the server holds the object of the talk's channel in `epoch`, signed by Ada's
/// device, with Bob's `pk` of it, once Bob's device took in its mailbox.
#[track_caller]
fn assert_adas(talk: &mut Talk, epoch: u64) {
    let (at, cloud) = (talk.at(), &talk.world.cloud);
    let object = held_one(cloud, &talk.ada, at, epoch);
    talk.played[0].take_in(cloud);
    let signing = cert_of(cloud, &talk.ada).body().signing;
    let key = pk_of(&talk.played[0], at.1, epoch);
    assert_object(&object, (at.1, epoch), key, (talk.ada.device, &signing));
}

/// A revision Ada's device signs adds Carol to the channel, and Ada's next run commits the Add of
/// Carol's device: epoch 2. The server then holds the object of epoch 2, signed by Ada's device,
/// whose `key` is the `pk` of epoch 2 that Carol's and Bob's devices derive.
#[test]
fn a_commit_s_sender_uploads_the_object_of_the_epoch_it_starts() {
    let mut talk = Talk::new(
        &[5_419_100_001, 5_419_100_002],
        &["ada-queue-key-sender", "bob-queue-key-sender"],
        false,
    );
    let mut carol = carol(&talk, 5_419_100_003, "carol-queue-key-sender");
    let (cloud, at) = (&talk.world.cloud, talk.at());
    adds_to_channel(cloud, &talk.ada, at, &carol.person);

    received(&talk.ada.machine);

    assert_adas(&mut talk, 2);
    carol.take_in(&talk.world.cloud);
    let object = held_one(talk.cloud(), &talk.ada, at, 2);
    assert!(
        object.body().key == pk_of(&carol, at.1, 2),
        "the object holds another key than Carol's pk of epoch 2"
    );
}

/// Ada's device created the channel's group with Bob's at epoch 1; after Ada's next run the server
/// holds the object of epoch 1 with Bob's `pk` of it. Bob's device commits an empty Commit, epoch
/// 2, and uploads no object: the server holds none of epoch 2. Ada's next run catches up and
/// uploads it: signed by Ada's device, with Bob's `pk` of epoch 2.
#[test]
fn a_member_device_uploads_after_its_catch_up_the_object_the_server_lacks() {
    let mut talk = Talk::new(
        &[5_419_110_001, 5_419_110_002],
        &["ada-queue-key-catch-up", "bob-queue-key-catch-up"],
        false,
    );
    let at = talk.at();
    received(&talk.ada.machine);
    assert_adas(&mut talk, 1);
    bob_commits_empty(&mut talk, 2);
    assert!(
        held(talk.cloud(), &talk.ada, at, 2).is_none(),
        "an object of epoch 2 before Ada's run"
    );

    received(&talk.ada.machine);

    assert_adas(&mut talk, 2);
}

/// Ada's run that commits the Add of Carol's device loses its upload of the object of epoch 2: the
/// front closes the connection at it. Her next two runs leave the object of epoch 2 on the server,
/// with Bob's `pk` of it; the front forwarded an object of epoch 2 to the server once.
#[test]
fn an_object_whose_upload_a_proxy_cuts_is_uploaded_once_by_a_later_run() {
    let mut talk = Talk::new(
        &[5_419_120_001, 5_419_120_002],
        &["ada-queue-key-cut", "bob-queue-key-cut"],
        true,
    );
    let carol = carol(&talk, 5_419_120_003, "carol-queue-key-cut");
    let at = talk.at();
    adds_to_channel(talk.cloud(), &talk.ada, at, &carol.person);
    let from = talk.front.queue_key_uploads().len();
    talk.front.cut_next_queue_key(at.1);

    // The run may fail, as its connection is closed.
    drop(run(&talk.ada.machine, &[RECV]));

    let cut = uploads_of(&talk.front, (at.1, 2), from);
    assert!(
        cut.iter().any(|upload| !upload.forwarded),
        "no object of epoch 2 cut: {cut:?}"
    );
    received(&talk.ada.machine);
    received(&talk.ada.machine);
    assert_adas(&mut talk, 2);
    let sent = uploads_of(&talk.front, (at.1, 2), from);
    let forwarded = sent.iter().filter(|upload| upload.forwarded).count();
    assert_eq!(
        forwarded, 1,
        "the object of epoch 2 not uploaded once: {sent:?}"
    );
}

/// A device the server lets in on Bob's account, its certificate chained to another master key,
/// joins the channel's group by itself: epoch 2, unclean. Ada's run commits its Remove: epoch 3.
/// Over that run and the next, Ada's machine uploads no object of epoch 2 and the server holds
/// none; it holds the object of epoch 3, with Bob's `pk` of it.
#[test]
fn no_object_is_uploaded_for_an_unclean_epoch() {
    let mut talk = Talk::new(
        &[5_419_130_001, 5_419_130_002],
        &["ada-queue-key-unclean", "bob-queue-key-unclean"],
        false,
    );
    let at = talk.at();
    let from = talk.front.queue_key_uploads().len();
    let intruder = intruder(talk.cloud(), &talk.played[0].person, "bob-queue-key-joiner");
    intruder_joins(talk.cloud(), &intruder, at);

    received(&talk.ada.machine);
    received(&talk.ada.machine);

    let sent = uploads_of(&talk.front, (at.1, 2), from);
    assert!(
        sent.is_empty(),
        "objects of the unclean epoch 2 sent: {sent:?}"
    );
    assert!(
        held(talk.cloud(), &talk.ada, at, 2).is_none(),
        "the server holds an object of the unclean epoch 2"
    );
    assert_adas(&mut talk, 3);
}

/// With the member agent in the group, Bob's device commits an empty Commit and uploads no object
/// of its epoch. Only the agent runs next: the server then holds that epoch's object, signed by the
/// agent's device, with Bob's `pk` of it.
#[test]
fn a_member_agent_uploads_the_object_the_server_lacks() {
    let mut bot = Bot::created(
        [5_419_140_001, 5_419_140_002],
        ["ada-queue-key-agent", "bob-queue-key-agent"],
    );
    let at = bot.talk.at();
    bot.brought_in();
    let bob = &mut bot.talk.played[0];
    let epoch = commits_empty(&bot.talk.world.cloud, bob, at);
    let ada = &bot.talk.ada;
    assert!(
        held(bot.talk.cloud(), ada, at, epoch).is_none(),
        "an object of epoch {epoch} before the agent's run"
    );

    bot.runner.received();

    let object = held_one(bot.talk.cloud(), ada, at, epoch);
    let (device, cert) = service_of(bot.talk.cloud(), ada);
    let key = pk_of(&bot.talk.played[0], at.1, epoch);
    assert_object(&object, (at.1, epoch), key, (device, &cert.body().signing));
}
