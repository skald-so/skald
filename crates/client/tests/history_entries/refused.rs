//! An entry the server refuses is not sent again, and the CLI says the message is not in the
//! history; an entry left behind while the channel moved on is never written again under a later
//! epoch's key, which devices that joined since could open.

use crate::chain::{bob_commits_empty, key_of};
use crate::entries::{
    ada_s_message, decoded, entries_of, entries_sent, said_not_in_history, talk_entries,
};
use crate::fixtures::{kill_once, send_args, strs};
use crate::front::Answers;
use crate::kit::RECV;
use crate::talk::Talk;
use crate::world::run;

/// The front turns the server's `history-stored` for the entry of Ada's message into a
/// `history-refused` with `problem`. Ada's `skald send` says, naming the message's `msg_id`, that
/// it is not in the history. With the answers forwarded, her next two runs send no entry of it
/// again.
fn refused_entry_is_not_sent_again(problem: &str, ids: &[u64; 2], logins: &[&str; 2]) {
    let talk = Talk::new(ids, logins, true);
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    talk.front
        .answer_history(Answers::Refuse(problem.to_owned()));

    let said = talk.send("a line the history refuses");

    let msg_id = ada_s_message(&talk, sends);
    let sent = entries_of(&talk.front, talk.channel, uploads, msg_id);
    assert!(!sent.is_empty(), "no entry of {msg_id} sent: {said:?}");
    assert!(
        !said_not_in_history(&said.shown(), msg_id).is_empty(),
        "does not say {msg_id} is not in the history: {said:?}"
    );
    talk.front.answer_history(Answers::Forward);
    let later = talk.front.uploads().len();

    for _ in 0..2 {
        drop(run(&talk.ada.machine, &[RECV]));
    }

    let again = entries_of(&talk.front, talk.channel, later, msg_id);
    assert!(again.is_empty(), "the refused entry sent again: {again:?}");
}

#[test]
fn refused_entry_is_not_sent_again_and_the_cli_says_the_message_is_not_in_the_history() {
    refused_entry_is_not_sent_again(
        "not-a-member",
        &[4_165_004_001, 4_165_004_002],
        &["ada-entry-refused", "bob-sees-refusal"],
    );
}

/// A problem the client does not know is a refusal too ("Uploads").
#[test]
fn entry_refused_with_a_problem_the_client_does_not_know_is_not_sent_again() {
    refused_entry_is_not_sent_again(
        "some-problem-of-a-later-version",
        &[4_165_004_101, 4_165_004_102],
        &["ada-entry-refused-unknown", "bob-sees-unknown-refusal"],
    );
}

/// The front loses each `stored`, and the test kills Ada's `skald send` once one is lost: her
/// message, of epoch 1, is stored, and its entry is not uploaded. Bob's device commits two empty
/// Commits: epochs 2 and 3. Ada's next run says, naming the message's `msg_id`, that it is not in
/// the history; every entry of it her machine sent is of epoch 1 and opens with Bob's key of epoch
/// 1, none under another key. The server holds no entry, and a later run sends none.
#[test]
fn entry_left_two_commits_behind_is_not_in_the_history_and_not_written_under_another_key() {
    let mut talk = Talk::new(
        &[4_165_005_001, 4_165_005_002],
        &["ada-left-behind", "bob-moves-on-twice"],
        true,
    );
    let (sends, uploads) = (talk.front.sent().len(), talk.front.uploads().len());
    let channel = talk.channel;
    talk.front.answer(Answers::Lose);
    let args = send_args(channel, "a line written before two Commits");
    let started = talk.ada.machine.start(&strs(&args));
    let front = &talk.front;
    kill_once(started, "a lost stored", || front.lost() > 0);
    let msg_id = ada_s_message(&talk, sends);
    let before = entries_sent(&talk.front, channel, uploads);
    assert!(
        before.is_empty(),
        "an entry sent before its message was stored: {before:?}"
    );
    talk.front.answer(Answers::Forward);
    bob_commits_empty(&mut talk, 2);
    bob_commits_empty(&mut talk, 3);

    let next = run(&talk.ada.machine, &[RECV]);

    assert!(
        !said_not_in_history(&next.shown(), msg_id).is_empty(),
        "does not say {msg_id} is not in the history: {next:?}"
    );
    let bob_s_key = key_of(&talk.played[0], channel, 1);
    for one in entries_of(&talk.front, channel, uploads, msg_id) {
        let entry = decoded(&one.bytes);
        assert_eq!(
            entry.epoch, 1,
            "an entry of {msg_id} of another epoch: {entry:?}"
        );
        assert!(
            bob_s_key.open_entry(&entry).is_ok(),
            "an entry of {msg_id} under another key than epoch 1's"
        );
    }
    let held = talk_entries(&talk);
    assert!(held.is_empty(), "the server holds an entry: {held:?}");
    let later = talk.front.uploads().len();
    drop(run(&talk.ada.machine, &[RECV]));
    let again = entries_of(&talk.front, channel, later, msg_id);
    assert!(
        again.is_empty(),
        "an entry of {msg_id} sent again: {again:?}"
    );
}
