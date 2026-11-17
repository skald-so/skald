//! An entry that did not reach the server, or whose answer did not reach the machine: the entry
//! waits in the outbox with its envelope, is uploaded once the envelope is stored, and is sent
//! again with the same bytes until the server answers, also by a later run, leaving one entry.

use crate::entries::{ada_s_message, assert_one_entry, entries_sent};
use crate::fixtures::{kill_once, send_args, strs};
use crate::front::Answers;
use crate::kit::received;
use crate::talk::Talk;

/// The text of the message whose answers are lost.
const LOST_LINE: &str = "a line whose answer is lost";

/// How Ada's `skald send` of [`LOST_LINE`] loses an answer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Lost {
    /// The front loses each `stored`, and the run ends by itself.
    Stored,
    /// The front loses each `stored`, and the test kills the run once one is lost.
    KilledBeforeStored,
    /// The front loses each `history-stored`, and the run ends by itself.
    EntryAnswer,
    /// The front loses each `history-stored`, and the test kills the run once it sent an entry.
    KilledAfterUpload,
}

/// Ada's `skald send` of [`LOST_LINE`] loses an answer as `lost` says. While its `stored` is lost,
/// the run sends no entry of the message. Ada's next run, `skald recv` with the answers forwarded,
/// leaves on the server one entry of the message under the key of epoch 1; each entry of it her
/// machine sent has the same bytes. With an entry's answer lost, the next run sent it again.
fn lost_answer_leaves_one_entry(lost: Lost, ids: &[u64; 2], logins: &[&str; 2]) {
    let mut talk = Talk::new(ids, logins, true);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    let channel = talk.channel;
    match lost {
        Lost::Stored | Lost::KilledBeforeStored => talk.front.answer(Answers::Lose),
        Lost::EntryAnswer | Lost::KilledAfterUpload => talk.front.answer_history(Answers::Lose),
    }
    let args = send_args(channel, LOST_LINE);
    let started = talk.ada.machine.start(&strs(&args));
    let front = &talk.front;
    match lost {
        Lost::Stored | Lost::EntryAnswer => drop(started.finish()),
        Lost::KilledBeforeStored => kill_once(started, "a lost stored", || front.lost() > 0),
        Lost::KilledAfterUpload => kill_once(started, "an entry sent", || {
            !entries_sent(front, channel, uploads).is_empty()
        }),
    }
    let msg_id = ada_s_message(&talk, sends);
    let first = entries_sent(&talk.front, channel, uploads);
    match lost {
        Lost::Stored | Lost::KilledBeforeStored => {
            assert!(talk.front.lost() > 0, "no stored was lost");
            assert!(
                first.is_empty(),
                "an entry sent before its message was stored: {first:?}"
            );
        }
        Lost::EntryAnswer | Lost::KilledAfterUpload => {
            assert!(!first.is_empty(), "no entry sent");
        }
    }
    talk.front.answer(Answers::Forward);
    talk.front.answer_history(Answers::Forward);
    let later = talk.front.uploads().len();

    received(&talk.ada.machine);

    assert_one_entry(&mut talk, uploads, (msg_id, 1));
    if matches!(lost, Lost::EntryAnswer | Lost::KilledAfterUpload) {
        let again = entries_sent(&talk.front, channel, later);
        assert!(
            again
                .iter()
                .any(|one| one.msg_id == msg_id && one.forwarded),
            "the entry whose answer was lost not sent again: {again:?}"
        );
    }
}

#[test]
fn entry_of_a_message_whose_stored_was_lost_is_uploaded_by_the_next_run() {
    lost_answer_leaves_one_entry(
        Lost::Stored,
        &[4_165_003_001, 4_165_003_002],
        &["ada-loses-entry-stored", "bob-awaits-entry"],
    );
}

#[test]
fn entry_of_a_cli_killed_before_its_message_was_stored_is_uploaded_by_the_next_run() {
    lost_answer_leaves_one_entry(
        Lost::KilledBeforeStored,
        &[4_165_003_101, 4_165_003_102],
        &["ada-killed-before-stored", "bob-awaits-killed-entry"],
    );
}

#[test]
fn entry_whose_answer_was_lost_is_sent_again_with_the_same_bytes() {
    lost_answer_leaves_one_entry(
        Lost::EntryAnswer,
        &[4_165_003_201, 4_165_003_202],
        &["ada-loses-entry-answer", "bob-awaits-entry-answer"],
    );
}

#[test]
fn entry_of_a_cli_killed_after_its_upload_is_sent_again_with_the_same_bytes() {
    lost_answer_leaves_one_entry(
        Lost::KilledAfterUpload,
        &[4_165_003_301, 4_165_003_302],
        &["ada-killed-after-upload", "bob-awaits-upload"],
    );
}
