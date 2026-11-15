//! Rejoining: Ada's device, which cannot process the channel's next Commit, or which a Commit
//! removed while it is admitted, rejoins with an external Commit from the latest GroupInfo, warning
//! in the second case; then all read each other. A Commit of an epoch it has passed is dropped.

use skald_crypto::MlsError;
use skald_proto::{ChannelId, RawEnvelope, SpaceId};

use crate::cloud::Cloud;
use crate::gaps::{bob_in, bob_sends};
use crate::joins::{assert_joined, drop_entry, kept, read_from, removal_warnings};
use crate::kit::{assert_members, group, received};
use crate::machine::Finished;
use crate::played::Played;
use crate::talk::{
    BOB_HOST, CAROL_HOST, Line, Talk, assert_shows_once, commit_sent, encrypted, stored,
};
use crate::world::now;

/// `played` commits `make`'s Commit in `at`'s channel, which the server stores; gives its envelope.
fn commits(
    cloud: &Cloud,
    played: &mut Played,
    at: (SpaceId, ChannelId),
    make: impl FnOnce(&mut Played) -> Result<Vec<u8>, MlsError>,
) -> RawEnvelope {
    let commit = make(played).expect("a Commit");
    kept(cloud, played, at, &commit)
}

/// Carol's device, the talk's second, says `text` with `counter` in the talk's channel.
fn carol_sends(talk: &mut Talk, counter: u64, text: &str) {
    let at = talk.at();
    let from = talk.address(&talk.played[1].person);
    let envelope = encrypted(
        &mut talk.played[1],
        at,
        at.1,
        &Line::new(&from, counter, text),
    );
    stored(&talk.world.cloud, &talk.played[1], &envelope);
}

/// Bob's and Carol's devices send a message each, which Ada's `skald recv` shows once each; Ada's
/// `skald send` sends one, which each of their devices reads from Ada's.
#[track_caller]
fn all_read_each_other(talk: &mut Talk, counter: u64, says: [&str; 3]) {
    let (from_bob, channel, _) = bob_in(talk);
    let from_carol = talk.address(&talk.played[1].person);
    bob_sends(talk, counter, says[0]);
    carol_sends(talk, counter, says[1]);

    let shown = received(&talk.ada.machine).shown();

    assert_shows_once(&shown, says[0], (channel, &from_bob, BOB_HOST));
    assert_shows_once(&shown, says[1], (channel, &from_carol, CAROL_HOST));
    let said = talk.send(says[2]);
    assert!(said.success, "{said:?}");
    let ada = talk.ada.device;
    for played in &mut talk.played {
        played.take_in(&talk.world.cloud);
        assert_eq!(
            read_from(played, channel, says[2]),
            [ada],
            "{} does not read Ada's message once",
            played.device()
        );
    }
}

/// Fails unless Ada's device rejoined the talk's channel in the run `ran`, after the front's first
/// `from` sends: it sent a Commit there, and uploaded the GroupInfo of `epoch`, which holds each of
/// the talk's devices once; Bob's and Carol's devices get Ada's as the joiner, which removes its
/// old leaf if `old`, and hold each device once.
#[track_caller]
fn assert_rejoined(talk: &mut Talk, (ran, from): (&Finished, usize), epoch: u64, old: bool) {
    let (ada, channel) = (talk.ada.device, talk.channel);
    assert!(
        commit_sent(&talk.front, channel, from),
        "no external Commit sent: {ran:?}"
    );
    let mut devices = vec![ada];
    devices.extend(talk.played.iter().map(Played::device));
    let cloud = &talk.world.cloud;
    let held = group(cloud, &talk.ada.token, talk.space.id, channel);
    held.assert_is(epoch, &devices);
    assert_eq!(held.signer, ada, "the GroupInfo's signer");
    for played in &mut talk.played {
        played.take_in(cloud);
        assert_joined(played, channel, ada, old);
        assert_members(&played.members(channel), &devices);
    }
}

/// Bob's device commits twice in the channel; the first Commit never reaches Ada's device. Ada's
/// `skald recv` fails on the second and rejoins: the group is at epoch 4 with one leaf of each
/// device, as the GroupInfo Ada's device uploads shows, and Bob's and Carol's devices get Ada's as
/// the joiner that removes its old leaf. All read each other. The front then puts the second Commit
/// into Ada's catch-up again: Ada's next run sends no Commit in the channel, the group stays, and
/// Bob's next message shows.
#[test]
fn device_that_missed_a_commit_rejoins_and_drops_a_commit_of_an_epoch_it_passed() {
    let mut talk = Talk::new(
        &[4_138_083_001, 4_138_083_002, 4_138_083_003],
        &["ada-misses-commit", "bob-commits-twice", "carol-gets-both"],
        false,
    );
    let (at, channel) = (talk.at(), talk.channel);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    let empty = |bob: &mut Played| bob.mls.commit_empty(channel, b"", now());
    let missed = commits(cloud, &mut talk.played[0], at, empty);
    drop_entry(cloud, ada, &missed);
    talk.played[1].take_in(cloud);
    let next = commits(cloud, &mut talk.played[0], at, empty);
    talk.played[1].take_in(cloud);
    let from = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert_rejoined(&mut talk, (&ran, from), 4, true);
    all_read_each_other(
        &mut talk,
        1,
        [
            "rejoined fine, the deploy log looks clean",
            "rollback plan is in the runbook",
            "thanks, I see both of you again",
        ],
    );
    talk.front.inject(ada, vec![next]);
    let from = talk.front.sent().len();

    let again = received(&talk.ada.machine);

    assert_eq!(
        talk.front.injected(),
        1,
        "the Commit was not put in: {again:?}"
    );
    assert!(
        !commit_sent(&talk.front, channel, from),
        "Ada's device commits on a Commit of an epoch it passed: {again:?}"
    );
    let still = group(&talk.world.cloud, &talk.ada.token, at.0, channel);
    assert_eq!(
        (still.epoch, still.signer),
        (4, ada),
        "the group's epoch and its GroupInfo's signer"
    );
    // The front's push took the `seq` the server gives Ada's device next, which her cursor now
    // covers: the server moves past it, so that Bob's next message reaches her.
    talk.world.cloud.execute(&format!(
        "UPDATE mailboxes SET last_seq = last_seq + 1 WHERE device_id = '{ada}'::uuid"
    ));
    let (from_bob, ..) = bob_in(&talk);
    let after = "the replayed commit changed nothing";
    bob_sends(&mut talk, 2, after);
    let shown = received(&talk.ada.machine).shown();
    assert_shows_once(&shown, after, (channel, &from_bob, BOB_HOST));
}

/// Bob's device, which a proxy keeps on a revision without Ada in the channel, commits the Remove
/// of Ada's device, which the latest revision admits. Ada's `skald recv` warns that a Commit
/// removed it from the channel and rejoins: the group is at epoch 3 with each device once, as the
/// GroupInfo Ada's device uploads shows, and Bob's and Carol's devices get Ada's as the joiner. All
/// read each other.
#[test]
fn device_removed_while_admitted_rejoins_and_warns() {
    let mut talk = Talk::new(
        &[4_138_083_101, 4_138_083_102, 4_138_083_103],
        &[
            "ada-wrongly-removed",
            "bob-on-old-revision",
            "carol-stays-on",
        ],
        false,
    );
    let (at, channel) = (talk.at(), talk.channel);
    let ada = talk.ada.device;
    let cloud = &talk.world.cloud;
    commits(cloud, &mut talk.played[0], at, |bob| {
        bob.mls.remove_devices(channel, &[ada], b"", now())
    });
    talk.played[1].take_in(cloud);
    let from = talk.front.sent().len();

    let ran = received(&talk.ada.machine);

    assert!(
        !removal_warnings(&ran, channel).is_empty(),
        "no warning of the removal: {ran:?}"
    );
    assert_rejoined(&mut talk, (&ran, from), 3, false);
    all_read_each_other(
        &mut talk,
        1,
        [
            "sorry, my device was a revision behind",
            "no harm done, ada is back",
            "back in, reading you both",
        ],
    );
}
