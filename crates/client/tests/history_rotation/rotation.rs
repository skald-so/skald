//! Rotation on schedule: an epoch older than the policy's `history` gets an empty Commit at Ada's
//! next run, and Ada and Bob talk in the new epoch; a younger epoch gets none; an epoch that ages
//! past the period before `skald send` is rotated before the message goes out; an epoch the local
//! state holds no entry time of counts from the run that finds it so.

use skald_proto::Envelope;

use crate::aged::{
    DAY, HOUR, ahead, assert_no_commit, assert_one_commit, assert_one_commit_each,
    assert_rotated_once, epoch_of, epochs, forget_entered, received_ahead, with_period,
};
use crate::fixtures::sent_after;
use crate::kit::received;
use crate::talk::{Talk, is_commit, is_message, one_message_sent};

/// What Ada and Bob say in the new epoch.
const ADA_SAYS: &str = "the rotated key still reads this";
const BOB_SAYS: &str = "and this reply comes through too";
const SENT_AGED: &str = "said once the epoch was two days old";

/// A revision of Ada's device sets the period to a day, and Ada's `skald recv` accepts it with the
/// epochs just made: it sends no Commit. Two days later, by Ada's clock, her next `skald recv`
/// commits one empty Commit in each channel: the server holds the next epoch with the same
/// devices, and Bob's device enters it from Ada's Commit, with a new history key. Ada and Bob read
/// each other in it, and Ada's runs then send no other Commit.
#[test]
fn an_epoch_older_than_the_period_gets_an_empty_commit_at_the_next_run() {
    let mut talk = Talk::new(
        &[4_162_201_001, 4_162_201_002],
        &["ada-rotates-history", "bob-sees-rotation"],
        false,
    );
    with_period(&talk, DAY);
    let before = epochs(&talk);
    let from = talk.front.sent().len();
    received(&talk.ada.machine);
    assert_no_commit(&talk, before, from);
    talk.ada = ahead(talk.ada, 2 * DAY);

    received(&talk.ada.machine);

    assert_rotated_once(&mut talk, (before, from), (ADA_SAYS, BOB_SAYS));
}

/// With a period of a day, Ada's `skald recv` 23 hours after the epochs began sends no Commit and
/// leaves them as they are; 25 hours after, it commits one empty Commit in each channel.
#[test]
fn an_epoch_younger_than_the_period_is_not_rotated_until_it_is_older() {
    let mut talk = Talk::new(
        &[4_162_202_001, 4_162_202_002],
        &["ada-waits-a-day", "bob-waits-a-day"],
        false,
    );
    with_period(&talk, DAY);
    let before = epochs(&talk);
    let from = talk.front.sent().len();
    talk.ada = ahead(talk.ada, 23 * HOUR);

    received(&talk.ada.machine);

    assert_no_commit(&talk, before, from);
    talk.ada = ahead(talk.ada, 25 * HOUR);
    received(&talk.ada.machine);
    assert_one_commit_each(&talk, before, from);
}

/// Ada's local state holds no time her device entered the epochs, as a client from before it kept
/// one leaves it. With a period of a day, her `skald recv` two days on counts the epochs from then
/// and sends no Commit; two days later still, her next run commits one empty Commit in each
/// channel.
#[test]
fn an_epoch_without_an_entry_time_in_the_local_state_is_rotated_once_the_period_passes() {
    let mut talk = Talk::new(
        &[4_162_211_001, 4_162_211_002],
        &["ada-older-state", "bob-older-state"],
        false,
    );
    with_period(&talk, DAY);
    forget_entered(&talk.ada);
    let (before, from) = (epochs(&talk), talk.front.sent().len());
    talk = received_ahead(talk, 2 * DAY);
    assert_no_commit(&talk, before, from);

    talk = received_ahead(talk, 4 * DAY);

    assert_rotated_once(&mut talk, (before, from), (ADA_SAYS, BOB_SAYS));
}

/// With a period of a day, Ada's `skald send` two days after the channel's epoch began commits an
/// empty Commit there before it sends: the Commit goes out before the message, which is encrypted
/// in the new epoch, and Bob's device reads it there.
#[test]
fn an_aged_epoch_is_rotated_before_skald_send_sends_in_the_channel() {
    let mut talk = Talk::new(
        &[4_162_203_001, 4_162_203_002],
        &["ada-sends-late", "bob-reads-late"],
        false,
    );
    with_period(&talk, DAY);
    let before = epochs(&talk)[1];
    let from = talk.front.sent().len();
    talk.ada = ahead(talk.ada, 2 * DAY);

    let said = talk.send(SENT_AGED);

    assert!(said.success, "{said:?}");
    let channel = talk.channel;
    assert_one_commit(&talk, (channel, before + 1), from);
    let in_channel: Vec<_> = sent_after(&talk.front, from)
        .into_iter()
        .map(|sent| sent.envelope)
        .filter(|envelope| envelope.envelope().header.channel == channel)
        .collect();
    let first = |of: fn(&Envelope) -> bool, what: &str| {
        let at = in_channel
            .iter()
            .position(|envelope| of(envelope.envelope()));
        at.unwrap_or_else(|| panic!("no {what} in {channel}"))
    };
    let (commit, message) = (first(is_commit, "Commit"), first(is_message, "message"));
    assert!(commit < message, "the message went out before the Commit");
    let message = &in_channel[message];
    assert_eq!(epoch_of(message), before + 1, "the message's epoch");
    let msg_id = one_message_sent(&talk.front, channel, from);
    talk.bob_reads_from_ada(&[(msg_id, 1, SENT_AGED)]);
}
