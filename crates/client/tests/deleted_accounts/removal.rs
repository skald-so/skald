//! Removing a deleted account's devices: the first member device of each channel to learn of the
//! deletion removes them by a Commit before it sends there; a device that processes that Commit
//! commits nothing more; each tells its user once and marks the member deleted; a running helper
//! does it on the `space-notice`.

use std::collections::BTreeSet;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{DeviceId, Role};

use crate::cloud::WAIT;
use crate::fixtures::{await_running, send};
use crate::helped::{Helped, WEBSOCKET};
use crate::kit::{RECV, assert_members, group, received};
use crate::scene::{
    PUSHED, Scene, assert_marked_deleted, assert_read_from, assert_told_once, commits_sent,
    delete_account, epoch_of, one_message_sent, reported_deleted,
};
use crate::talk::messages_sent;
use crate::world::{ACME, members, now};

/// What Bob says after Ada's deletion; neither text is anywhere else.
const AFTER: [&str; 2] = [
    "the budget review moved to thursday",
    "please re-share the staging credentials",
];

/// The flag of `skald recv` that waits for what is to come.
const WAIT_FLAG: &str = "--wait";

/// Fails unless `played`'s changes of `channel` hold one Commit of `sender` that removes `removed`
/// and adds nothing.
#[track_caller]
fn assert_removed_by(
    changes: &[skald_crypto::GroupChange],
    sender: DeviceId,
    removed: &[DeviceId],
) {
    let wanted: BTreeSet<DeviceId> = removed.iter().copied().collect();
    let of_sender: Vec<_> = changes
        .iter()
        .filter(|change| change.sender == sender)
        .collect();
    let matching = of_sender.iter().filter(|change| {
        let gone: BTreeSet<DeviceId> = change.removed.iter().map(|one| one.device).collect();
        gone == wanted && change.added.is_empty()
    });
    assert_eq!(
        matching.count(),
        1,
        "not one Commit of {sender} removing {removed:?}: {changes:?}"
    );
}

/// Ada, a member of `acme`, deletes her account. Bob's next run, `skald send` in the channel,
/// removes Ada's two devices from both of the space's channels, each by one Commit, and then sends
/// his line in the channel, in a later epoch than that Commit. The server's GroupInfo of each
/// channel then holds Bob's and Carol's devices alone; Carol's device enters each Commit, which
/// removes Ada's two devices and adds none, and reads Bob's line. Bob's run tells that Ada is
/// deleted, his next `skald recv` does not tell it again, and `skald members acme` marks Ada
/// deleted, and neither Bob nor Carol.
#[test]
fn first_member_device_to_learn_of_a_deletion_removes_its_devices_then_sends() {
    let mut scene = Scene::new(
        [4_305_101_001, 4_305_101_002, 4_305_101_003],
        [
            "bob-learns-first",
            "ada-deletes-herself",
            "carol-sees-removal",
        ],
        Role::Member,
    );
    let cloud = &scene.world.cloud;
    let ada = scene.ada_devices();
    let handle = scene.ada().handle.to_string();
    delete_account(cloud, scene.ada());
    let from = scene.front.sent().len();

    let sent = send(&scene.bob.machine, scene.channel, AFTER[0]);

    assert!(sent.success, "{sent:?}");
    let (bob, carol) = (scene.bob.device, scene.carol.device());
    scene.assert_groups(&[bob, carol]);
    for channel in scene.channels() {
        let removals = commits_sent(&scene.front, channel, from);
        assert_eq!(removals.len(), 1, "not one Commit in {channel}: {sent:?}");
    }
    let removal = commits_sent(&scene.front, scene.channel, from)[0];
    let said = messages_sent(&scene.front, scene.channel, from);
    assert_eq!(said.len(), 1, "not one message sent: {sent:?}");
    let message = said[0].envelope.envelope();
    assert!(
        epoch_of(&message.payload) > removal,
        "the line was sent in epoch {}, not after the removal's {removal}",
        epoch_of(&message.payload)
    );
    let msg_id = message.header.msg_id;
    scene.carol.take_in(cloud);
    for channel in scene.channels() {
        assert_removed_by(&scene.carol.changes_of(channel), bob, &ada);
        assert_members(&scene.carol.members(channel), &[bob, carol]);
    }
    assert_read_from(&scene.carol, scene.channel, msg_id, bob);
    let later = received(&scene.bob.machine);
    assert_told_once(&[&sent, &later], &handle);
    let others = [
        scene.bob.handle.as_str(),
        scene.carol.person.handle.as_str(),
    ];
    assert_marked_deleted(&members(&scene.bob.machine, ACME), &handle, &others);
}

/// Ada deletes her account while Bob's machine is offline. Carol's device, which the server tells
/// that Ada is deleted, comes online first and removes Ada's devices from both channels. Bob's next
/// `skald recv` commits nothing, and his `skald send` in the channel after it neither; the server's
/// GroupInfo of each channel holds Bob's and Carol's devices alone, and Carol reads Bob's line.
/// Bob's first run tells that Ada is deleted, his later one does not, and `skald members acme`
/// marks Ada deleted.
#[test]
fn first_member_device_online_removes_them_and_the_others_commit_nothing_more() {
    let mut scene = Scene::new(
        [4_305_101_101, 4_305_101_102, 4_305_101_103],
        [
            "bob-offline-at-first",
            "ada-gone-meanwhile",
            "carol-online-first",
        ],
        Role::Member,
    );
    let cloud = &scene.world.cloud;
    let (space, ada) = (scene.space.id, scene.ada_devices());
    let (account, handle) = (scene.ada().account, scene.ada().handle.to_string());
    delete_account(cloud, scene.ada());
    let reported = reported_deleted(cloud, &scene.carol.person.token, space);
    assert_eq!(reported, vec![account], "the server's report to Carol");
    for channel in scene.channels() {
        let at = (space, channel);
        let removal = (scene.carol.mls)
            .remove_devices(channel, &ada, &[], now())
            .expect("a Remove");
        scene.carol.commits(cloud, at, &removal);
        scene.carol.uploads_group_info(cloud, at);
    }
    let from = scene.front.sent().len();

    let ran = received(&scene.bob.machine);
    let sent = send(&scene.bob.machine, scene.channel, AFTER[1]);

    assert!(sent.success, "{sent:?}");
    scene.assert_no_commit(from, &[&ran, &sent]);
    let (bob, carol) = (scene.bob.device, scene.carol.device());
    scene.assert_groups(&[bob, carol]);
    let msg_id = one_message_sent(&scene.front, scene.channel, from);
    scene.carol.take_in(cloud);
    assert_read_from(&scene.carol, scene.channel, msg_id, bob);
    assert_told_once(&[&ran, &sent], &handle);
    assert_marked_deleted(&members(&scene.bob.machine, ACME), &handle, &[]);
}

/// Bob's `skald recv --wait` runs through his helper and has connected. Ada deletes her account:
/// with no other command, the server's GroupInfo of each channel soon holds Bob's and Carol's
/// devices alone, and the front saw the server send Bob's device a `space-notice` of `acme`.
#[test]
fn space_notice_makes_the_helper_remove_the_devices_with_no_command() {
    let scene = Scene::new(
        [4_305_101_201, 4_305_101_202, 4_305_101_203],
        [
            "bob-helper-running",
            "ada-deletes-while-he-waits",
            "carol-helped",
        ],
        Role::Member,
    );
    let cloud = scene.cloud();
    // Dropped first: it kills the helper before the machine's directory goes.
    let helped = Helped::new(&scene.bob.machine);
    let since = cloud.relay.mark();
    let mut waiting = helped.start(&[], &[RECV, WAIT_FLAG]);
    await_running(&mut waiting, "connection of the waiting run", || {
        cloud.count_sent(since, WEBSOCKET) > 0
    });
    // A moment for its catch-up to end.
    thread::sleep(PUSHED);

    delete_account(cloud, scene.ada());

    let (bob, carol) = (scene.bob.device, scene.carol.device());
    let mut wanted = vec![bob, carol];
    wanted.sort();
    let removed = || {
        scene.channels().iter().all(|channel| {
            let mut held = group(cloud, &scene.bob.token, scene.space.id, *channel).members;
            held.sort();
            held == wanted
        })
    };
    let deadline = Instant::now() + WAIT;
    while !removed() {
        let gone = waiting.exited();
        assert!(
            !gone && Instant::now() < deadline,
            "Ada's devices were not removed (exited: {gone}): {}",
            waiting.shown()
        );
        thread::sleep(Duration::from_millis(100));
    }
    assert!(
        scene.front.notices_to(bob).contains(&scene.space.id),
        "no notice of acme reached Bob's device"
    );
    waiting.kill();
}
