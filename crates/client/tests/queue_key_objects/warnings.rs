//! A member device that finds the server's queue key object with another `key` than the `pk` it
//! derives warns its user, once per epoch; whether a member device uploaded that object first or a
//! proxy swapped it.

use skald_proto::Frame;

use crate::chain::bob_commits_empty;
use crate::kit::received;
use crate::objects::{object_of, object_request, pk_of, queue_key_warnings, wrong_key};
use crate::scene::change_field;
use crate::talk::Talk;

/// Bob's device uploads `key` as the object of the talk's channel in `epoch`, signed by it; fails
/// unless the server takes it.
#[track_caller]
fn bob_uploads(talk: &Talk, epoch: u64, key: [u8; 32]) -> Vec<u8> {
    let (bob, at) = (&talk.played[0], talk.at());
    let object = object_of((at.1, epoch), key, (bob.device(), &bob.person.key));
    let answer = bob.connect(talk.cloud()).upload_queue_key(1, at, &object);
    assert_eq!(
        answer,
        Frame::HistoryStored { upload: 1 },
        "Bob's object of epoch {epoch}"
    );
    object
}

/// Bob's device commits an empty Commit, epoch 2, and first uploads an object of epoch 2 with a key
/// no device derives. Ada's next run warns of the channel's queue key; the run after it does not
/// warn again. Bob's device commits another, epoch 3, and uploads its object with the `pk` of
/// epoch 3: Ada's next run gives no such warning.
#[test]
fn a_wrong_object_the_server_keeps_makes_a_member_device_warn_once() {
    let mut talk = Talk::new(
        &[5_419_200_001, 5_419_200_002],
        &["ada-queue-key-warned", "bob-queue-key-wrong"],
        false,
    );
    received(&talk.ada.machine);
    bob_commits_empty(&mut talk, 2);
    bob_uploads(&talk, 2, wrong_key());

    let warned = received(&talk.ada.machine);

    assert!(
        !queue_key_warnings(&warned).is_empty(),
        "no warning of the wrong queue key: {warned:?}"
    );
    let again = received(&talk.ada.machine);
    assert_eq!(
        queue_key_warnings(&again),
        Vec::<String>::new(),
        "a second warning of epoch 2"
    );
    bob_commits_empty(&mut talk, 3);
    bob_uploads(&talk, 3, pk_of(&talk.played[0], talk.channel, 3));
    let clean = received(&talk.ada.machine);
    assert_eq!(
        queue_key_warnings(&clean),
        Vec::<String>::new(),
        "a warning of the genuine object of epoch 3"
    );
}

/// Bob's device commits an empty Commit, epoch 2, and uploads its object with the `pk` of epoch 2.
/// The relay gives in its place an object of epoch 2 that Bob's device signed with a key no device
/// derives: Ada's next run warns of the channel's queue key.
#[test]
fn an_object_a_proxy_swaps_makes_a_member_device_warn() {
    let mut talk = Talk::new(
        &[5_419_210_001, 5_419_210_002],
        &["ada-queue-key-swapped", "bob-queue-key-genuine"],
        false,
    );
    received(&talk.ada.machine);
    bob_commits_empty(&mut talk, 2);
    bob_uploads(&talk, 2, pk_of(&talk.played[0], talk.channel, 2));
    let (bob, at) = (&talk.played[0], talk.at());
    let swapped = object_of((at.1, 2), wrong_key(), (bob.device(), &bob.person.key));
    change_field(
        talk.cloud(),
        &object_request(at, 2),
        "queue_key",
        move |bytes| {
            bytes.clone_from(&swapped);
        },
    );

    let warned = received(&talk.ada.machine);

    assert!(
        !queue_key_warnings(&warned).is_empty(),
        "no warning of the swapped queue key: {warned:?}"
    );
}
