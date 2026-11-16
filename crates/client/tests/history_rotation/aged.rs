//! What the tests share: Ada's machine with its clock moved on, a period a fixture revision sets,
//! the epochs of the talk's channels and the Commits the front saw Ada's machine send there, what
//! Bob's device takes in of them, and messages both ways in an epoch.

use skald_crypto::MlsFraming;
use skald_proto::{ChannelId, RawEnvelope};

use crate::edits::envelopes_sent;
use crate::kit::{assert_members, group, latest, received};
use crate::played::Took;
use crate::talk::{
    BOB_HOST, Line, Talk, assert_shows_once, encrypted, is_commit, messages_sent, one_message_sent,
    stored,
};
use crate::world::{Person, next_with, store};

/// A day and an hour, in seconds.
pub const DAY: u64 = 86_400;
pub const HOUR: u64 = 3_600;

/// Drops from `person`'s local state when its device entered each epoch, as a client from before it
/// kept that leaves the state.
pub fn forget_entered(person: &Person) {
    let state = rusqlite::Connection::open(person.machine.state_dir().join("state.sqlite3"))
        .expect("the machine's local state");
    state
        .execute("DELETE FROM channel_epoch", [])
        .expect("forget the epochs' entry times");
}

/// The setting of the seconds the CLI adds to the system's clock.
pub const CLOCK_OFFSET: &str = "SKALD_CLOCK_OFFSET";

/// `person`, the runs of its machine from now on counting time `by` seconds ahead of the system's
/// clock.
pub fn ahead(mut person: Person, by: u64) -> Person {
    person.machine = person.machine.with_env(CLOCK_OFFSET, by.to_string());
    person
}

/// `talk` after Ada's `skald recv` with her clock `by` seconds ahead of the system's.
pub fn received_ahead(mut talk: Talk, by: u64) -> Talk {
    talk.ada = ahead(talk.ada, by);
    received(&talk.ada.machine);
    talk
}

/// Ada's device signs the revision after the latest of the talk's space, with the policy's
/// `history` `seconds` and no other change, stored on the server as a fixture.
pub fn with_period(talk: &Talk, seconds: u64) {
    let (cloud, ada) = (talk.cloud(), &talk.ada);
    let previous = latest(cloud, &ada.token, talk.space.id);
    let next = next_with(&previous, (ada.device, &ada.key), |body| {
        body.policy.history = seconds;
    });
    store(cloud, &ada.token, &next);
}

/// The talk's channels: its space's default channel, then its own.
pub fn talk_channels(talk: &Talk) -> [ChannelId; 2] {
    [talk.space.default, talk.channel]
}

/// The epoch of each of the talk's channels, as the server's GroupInfo gives it to Ada's session.
pub fn epochs(talk: &Talk) -> [u64; 2] {
    talk_channels(talk)
        .map(|channel| group(talk.cloud(), &talk.ada.token, talk.space.id, channel).epoch)
}

/// The talk's channels, each with the epoch after the one `before` gives it.
pub fn next_epochs(talk: &Talk, before: [u64; 2]) -> [(ChannelId, u64); 2] {
    let [default, own] = talk_channels(talk);
    [(default, before[0] + 1), (own, before[1] + 1)]
}

/// How many different Commits the front saw Ada's machine send in `channel` after its first `from`
/// sends: one sent again with the same bytes is no other one.
pub fn commits_sent(talk: &Talk, channel: ChannelId, from: usize) -> usize {
    envelopes_sent(&talk.front, channel, from)
        .iter()
        .filter(|sent| is_commit(sent.envelope()))
        .count()
}

/// Fails unless the front saw Ada's machine send no Commit in the talk's channels after its first
/// `from` sends, and the server holds each at its epoch of `before`.
#[track_caller]
pub fn assert_no_commit(talk: &Talk, before: [u64; 2], from: usize) {
    for channel in talk_channels(talk) {
        assert_eq!(
            commits_sent(talk, channel, from),
            0,
            "a Commit in {channel}"
        );
    }
    assert_eq!(epochs(talk), before, "an epoch moved on");
}

/// Fails unless the front saw Ada's machine send one Commit in `channel` after its first `from`
/// sends, and the server's GroupInfo shows the channel at `epoch` with Ada's and Bob's devices.
#[track_caller]
pub fn assert_one_commit(talk: &Talk, (channel, epoch): (ChannelId, u64), from: usize) {
    assert_eq!(
        commits_sent(talk, channel, from),
        1,
        "not one Commit of Ada's machine in {channel}"
    );
    let devices = [talk.ada.device, talk.played[0].device()];
    group(talk.cloud(), &talk.ada.token, talk.space.id, channel).assert_is(epoch, &devices);
}

/// As [`assert_one_commit`], in each of the talk's channels, at the epoch after the one `before`
/// gives it.
#[track_caller]
pub fn assert_one_commit_each(talk: &Talk, before: [u64; 2], from: usize) {
    for next in next_epochs(talk, before) {
        assert_one_commit(talk, next, from);
    }
}

/// Bob's device takes in its mailbox; fails unless, in each of the talk's channels, it entered one
/// epoch, the one after `before`'s, by an empty Commit of Ada's device: no device added or removed,
/// Ada's and Bob's devices in the group, and a history key other than that of the epoch before.
#[track_caller]
pub fn assert_bob_entered_by_empty_commits(talk: &mut Talk, before: [u64; 2]) {
    let next = next_epochs(talk, before);
    let ada = talk.ada.device;
    let bob = &mut talk.played[0];
    let took = bob.take_in(&talk.world.cloud);
    for (channel, epoch) in next {
        let changes: Vec<_> = took
            .iter()
            .filter_map(|one| match one {
                Took::Changed(of, change) if *of == channel => Some(change),
                _ => None,
            })
            .collect();
        assert_eq!(changes.len(), 1, "not one new epoch of {channel}: {took:?}");
        let change = changes[0];
        assert_eq!((change.epoch, change.sender), (epoch, ada), "{change:?}");
        assert!(
            change.added.is_empty() && change.removed.is_empty(),
            "not an empty Commit: {change:?}"
        );
        assert_members(&bob.members(channel), &[ada, bob.device()]);
        let key = |epoch: u64| {
            let held = bob.mls.history_key(channel, epoch);
            *held.expect("the epoch's history key").to_bytes()
        };
        assert_ne!(key(epoch - 1), key(epoch), "the history key of {channel}");
    }
}

/// The epoch of the MLS message in `envelope`.
#[track_caller]
pub fn epoch_of(envelope: &RawEnvelope) -> u64 {
    match MlsFraming::read(&envelope.envelope().payload) {
        Some(MlsFraming::Message { epoch, .. }) => epoch,
        _ => panic!("no MLS message"),
    }
}

/// In the talk's channel at `epoch`: Ada's machine says `ada_says`, its first message there, and
/// Bob's device reads it; Bob's device says `bob_says`, its first there, and Ada's `skald recv`
/// shows it once.
#[track_caller]
pub fn assert_talk_in(talk: &mut Talk, epoch: u64, (ada_says, bob_says): (&str, &str)) {
    let from = talk.front.sent().len();
    let said = talk.send(ada_says);
    assert!(said.success, "{said:?}");
    let msg_id = one_message_sent(&talk.front, talk.channel, from);
    let sent = messages_sent(&talk.front, talk.channel, from);
    assert_eq!(epoch_of(&sent[0].envelope), epoch, "Ada's message's epoch");
    talk.bob_reads_from_ada(&[(msg_id, 1, ada_says)]);

    let (at, cloud) = (talk.at(), &talk.world.cloud);
    let from_bob = talk.address(&talk.played[0].person);
    let bob = &mut talk.played[0];
    assert_eq!(bob.mls.epoch(at.1).expect("Bob's group"), epoch);
    let envelope = encrypted(bob, at, at.1, &Line::new(&from_bob, 1, bob_says));
    stored(cloud, bob, &envelope);
    let shown = received(&talk.ada.machine).shown();
    assert_shows_once(&shown, bob_says, (at.1, &from_bob, BOB_HOST));
}

/// Fails unless, since the front's first `from` sends, Ada's machine rotated each of the talk's
/// channels once from its epoch of `before`, as [`assert_one_commit_each`] and
/// [`assert_bob_entered_by_empty_commits`] say, Ada and Bob then say `says` to each other in the
/// talk's channel's new epoch, as [`assert_talk_in`] says, and Ada's runs sent no other Commit.
#[track_caller]
pub fn assert_rotated_once(talk: &mut Talk, (before, from): ([u64; 2], usize), says: (&str, &str)) {
    assert_one_commit_each(talk, before, from);
    assert_bob_entered_by_empty_commits(talk, before);
    assert_talk_in(talk, before[1] + 1, says);
    received(&talk.ada.machine);
    assert_one_commit_each(talk, before, from);
}
