//! A helper and a command of different versions: an older helper gives way to a newer command; an
//! older command does not use a newer helper.

use crate::gaps::{bob_in, bob_sends};
use crate::helped::{EXIT_LIMIT, Helped, VERSION, VERSION_SETTING, bob_reads, newer, older, times};
use crate::talk::{BOB_HOST, Talk, assert_shows_once};

/// What Ada's machine sends and Bob's device says; none is anywhere else.
const FIRST: &str = "helper test: version one of the report";
const SECOND: &str = "helper test: version two of the report";
const THIRD: &str = "helper test: version three of the report";
const BOB_SAYS: &str = "helper test: got the first version";

/// What an older command says when the `skald` on disk is the helper's version, and when it is
/// older than the helper.
const AGAIN: &str = "again";
const UPDATE: &str = "update";

/// A `skald send` acting as an older version than the copy's leaves a helper of that version
/// running; Bob's device then sends a message to the channel. A `skald send` of the copy's version
/// succeeds; the older helper exits and a new one runs. The next `skald recv` shows Bob's message
/// once, and Bob's device reads Ada's two messages, with the counters 1 and 2.
#[test]
fn newer_command_is_served_by_a_new_helper_of_its_version_and_nothing_is_lost() {
    let mut talk = Talk::new(
        &[4_146_041_001, 4_146_041_002],
        &["ada-updates-under-helper", "bob-talks-across-versions"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let older = older();
    let as_older = [(VERSION_SETTING, older.as_str())];
    let first = helped.send_with(&talk, &as_older, FIRST);
    assert!(first.success, "{first:?}");
    let old_helper = helped.one_helper();
    let (from_bob, channel, _) = bob_in(&talk);
    bob_sends(&mut talk, 1, BOB_SAYS);

    helped.sent(&talk, SECOND);

    helped.await_exit(old_helper, EXIT_LIMIT);
    let new_helper = helped.one_helper();
    let received = helped.received();
    assert_shows_once(&received.shown(), BOB_SAYS, (channel, &from_bob, BOB_HOST));
    assert_eq!(helped.helpers(), [new_helper], "not the new helper");
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// Ada's first `skald send` leaves a helper of the copy's version running. A `skald send` acting
/// as an older version fails, naming both versions and saying to run it again, as when its binary
/// was replaced by the helper's while it ran; it sends nothing, and the helper keeps running. A
/// `skald send` of the copy's version then goes through that helper, and Bob's device reads the
/// two messages sent, with the counters 1 and 2.
#[test]
fn older_command_beside_a_newer_helper_fails_saying_to_run_it_again() {
    let mut talk = Talk::new(
        &[4_146_042_001, 4_146_042_002],
        &["ada-runs-a-stale-binary", "bob-reads-past-stale"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    helped.sent(&talk, FIRST);
    let helper = helped.one_helper();
    let older = older();
    let before = talk.front.sent().len();

    let refused = helped.send_with(&talk, &[(VERSION_SETTING, older.as_str())], SECOND);

    assert!(!refused.success, "{refused:?}");
    assert_names_versions(&refused.shown(), &older, VERSION);
    assert!(
        refused.shown().to_lowercase().contains(AGAIN),
        "it does not say to run it again: {refused:?}"
    );
    assert_eq!(talk.front.sent().len(), before, "it sent something");
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
    helped.sent(&talk, THIRD);
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    bob_reads(&mut talk, &[FIRST, THIRD]);
}

/// A `skald send` acting as a newer version than the copy's leaves a helper of that version
/// running. A `skald send` of the copy's version, which is also the version on disk, fails, naming
/// both versions and saying to update `skald`; it sends nothing, and the helper keeps running.
#[test]
fn command_older_than_the_helper_and_the_skald_on_disk_says_to_update_it() {
    let talk = Talk::new(
        &[4_146_043_001, 4_146_043_002],
        &["ada-has-an-old-install", "bob-waits-for-update"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let newer = newer();
    let first = helped.send_with(&talk, &[(VERSION_SETTING, newer.as_str())], FIRST);
    assert!(first.success, "{first:?}");
    let helper = helped.one_helper();
    let before = talk.front.sent().len();

    let refused = helped.send_with(&talk, &[], SECOND);

    assert!(!refused.success, "{refused:?}");
    assert_names_versions(&refused.shown(), VERSION, &newer);
    assert!(
        refused.shown().to_lowercase().contains(UPDATE),
        "it does not say to update skald: {refused:?}"
    );
    assert_eq!(talk.front.sent().len(), before, "it sent something");
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
}

/// Fails unless `shown` names the command's version `command` and the helper's `helper`.
#[track_caller]
fn assert_names_versions(shown: &str, command: &str, helper: &str) {
    for version in [command, helper] {
        assert!(
            times(shown, version) > 0,
            "the version {version} is not named: {shown}"
        );
    }
}
