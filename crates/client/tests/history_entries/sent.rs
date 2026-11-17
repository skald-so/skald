//! Each message `skald send` sends in a clean epoch leaves one entry, signed by the sending device,
//! under the key of the epoch it encrypts the message in.

use crate::chain::bob_commits_empty;
use crate::entries::{ada_s_message, assert_ada_s_entry, entries_sent, msg_id_in, talk_entries};
use crate::talk::Talk;

/// Ada's `skald send` sends two messages in the channel at epoch 1. The server holds two entries,
/// one of each: of epoch 1, each opens with Bob's key of epoch 1 and holds, signed by Ada's device,
/// her device as `sender`, the channel, the message's `msg_id` and the content Bob's device reads.
/// Each `history` frame of an entry names the `msg_id` of the message it holds.
#[test]
fn each_message_sent_leaves_one_entry_signed_by_its_device_under_its_epoch_s_key() {
    let mut talk = Talk::new(
        &[4_165_001_001, 4_165_001_002],
        &["ada-writes-history", "bob-reads-history"],
        false,
    );
    let uploads = talk.front.uploads().len();
    let mut sent = Vec::new();
    for text in ["first line for the history", "second line for the history"] {
        let from = talk.front.sent().len();

        let ran = talk.send(text);

        assert!(ran.success, "{ran:?}");
        sent.push(ada_s_message(&talk, from));
    }

    let held = talk_entries(&talk);
    assert_eq!(held.len(), 2, "not one entry per message: {held:?}");
    for msg_id in sent {
        let of: Vec<&Vec<u8>> = (held.iter())
            .filter(|bytes| msg_id_in(&talk, bytes, 1) == Some(msg_id))
            .collect();
        assert_eq!(of.len(), 1, "not one entry of {msg_id}: {held:?}");
        let bytes = of[0].clone();
        assert_ada_s_entry(&mut talk, &bytes, (msg_id, 1));
        let frames: Vec<_> = (entries_sent(&talk.front, talk.channel, uploads).into_iter())
            .filter(|upload| upload.bytes == bytes)
            .collect();
        assert!(
            !frames.is_empty() && frames.iter().all(|upload| upload.msg_id == msg_id),
            "the entry of {msg_id} not uploaded naming it: {frames:?}"
        );
    }
}

/// Bob's device commits an empty Commit: epoch 2. Ada's `skald send` encrypts its message in epoch
/// 2: the server holds its one entry, of epoch 2, which opens with Bob's key of epoch 2.
#[test]
fn message_sent_after_a_commit_leaves_its_entry_under_the_new_epoch_s_key() {
    let mut talk = Talk::new(
        &[4_165_002_001, 4_165_002_002],
        &["ada-writes-after-commit", "bob-rotates-first"],
        false,
    );
    bob_commits_empty(&mut talk, 2);
    let from = talk.front.sent().len();

    let ran = talk.send("a line in the second epoch");

    assert!(ran.success, "{ran:?}");
    let msg_id = ada_s_message(&talk, from);
    let held = talk_entries(&talk);
    assert_eq!(held.len(), 1, "not one entry: {held:?}");
    assert_ada_s_entry(&mut talk, &held[0], (msg_id, 2));
}
