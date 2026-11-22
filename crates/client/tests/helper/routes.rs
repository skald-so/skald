//! Which runs start a helper, and what a run does when its helper cannot start, is busy, or
//! stands in the way of a sign-in.

use std::fs;
use std::time::Instant;

use crate::fixtures::{SEND, await_running};
use crate::front::Answers;
use crate::helped::{
    EXIT_LIMIT, Helped, IDLE_SETTING, LONG_IDLE, VERSION, VERSION_SETTING, bob_reads, newer,
    timeout_of, times,
};
use crate::kit::RECV;
use crate::machine::{SERVER, SIGN_IN};
use crate::talk::Talk;
use crate::world::{ACME, ADD_RULE, dir_of, path};

/// What Ada's machine sends; none is anywhere else.
const FIRST: &str = "helper test: the backup finished";
const SECOND: &str = "helper test: the backup was verified";

/// The request timeout of a run that must not wait it out, in seconds.
const LONG_TIMEOUT: u64 = 120;

/// The request timeout of the run that waits for a busy helper, in seconds, and that of the run
/// that keeps it busy, which the test ends well before.
const BUSY_WAIT: &str = "3";
const BUSY_FOR: &str = "60";

/// What a run says of a helper busy with another command.
const BUSY: &str = "busy";

/// `skald add-rule`, which does not connect as the device, starts no helper.
#[test]
fn command_that_does_not_connect_as_the_device_starts_no_helper() {
    let talk = Talk::new(
        &[4_146_071_001, 4_146_071_002],
        &["ada-adds-a-rule-alone", "bob-sees-no-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let folder = dir_of(&talk.ada.machine, "work/acme-alone");

    let added = helped.run(&[ADD_RULE, path(&folder), ACME]);

    assert!(added.success, "{added:?}");
    helped.assert_none();
}

/// With the helper's directory in the state directory taken by a file, no helper can start. Ada's
/// `skald send`, with a request timeout of [`LONG_TIMEOUT`], succeeds without waiting it out: it
/// runs one-shot once its helper has exited. No helper runs, and Bob's device reads the message.
#[test]
fn command_whose_helper_cannot_start_runs_one_shot() {
    let mut talk = Talk::new(
        &[4_146_072_001, 4_146_072_002],
        &["ada-cannot-start-one", "bob-reads-one-shot-run"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    fs::write(talk.ada.machine.state_dir().join("helper"), b"").expect("take the helper's place");
    let timeout = LONG_TIMEOUT.to_string();
    let started = Instant::now();

    let sent = helped.send_with(&talk, &timeout_of(&timeout), FIRST);

    assert!(sent.success, "{sent:?}");
    assert!(
        started.elapsed().as_secs() < LONG_TIMEOUT,
        "it waited its timeout out: {sent:?}"
    );
    helped.assert_none();
    bob_reads(&mut talk, &[FIRST]);
}

/// While the helper waits for the server's answer to Ada's `skald send`, which the front loses,
/// a `skald recv` with a request timeout of [`BUSY_WAIT`] fails, saying the helper is busy. With
/// the answers forwarded again, the waiting `skald send` succeeds through the same helper, and
/// Bob's device reads both messages.
#[test]
fn command_waiting_for_a_busy_helper_past_its_timeout_fails_saying_so() {
    let mut talk = Talk::new(
        &[4_146_073_001, 4_146_073_002],
        &["ada-keeps-it-busy", "bob-reads-after-busy"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let helper = helped.helper_after_sending(&talk, &[], FIRST);
    talk.front.answer(Answers::Lose);
    let lost = talk.front.lost();
    let channel = talk.channel.to_string();
    let mut busy = helped.start(
        &timeout_of(BUSY_FOR),
        &[SEND, "--all", "--channel", &channel, SECOND],
    );
    await_running(&mut busy, "lost stored answer", || talk.front.lost() > lost);

    let waited = helped.run_with(&timeout_of(BUSY_WAIT), &[RECV]);

    assert!(!waited.success, "{waited:?}");
    assert!(waited.shown().contains(BUSY), "{waited:?}");
    talk.front.answer(Answers::Forward);
    let sent = busy.finish();
    assert!(sent.success, "{sent:?}");
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// With a helper running that would not exit idle meanwhile, Ada's `skald sign-in` again succeeds,
/// and the helper exits. The next `skald send` starts a new helper, and Bob's device reads both
/// messages.
#[test]
fn sign_in_stops_a_running_helper() {
    let mut talk = Talk::new(
        &[4_146_074_001, 4_146_074_002],
        &["ada-signs-in-again", "bob-reads-after-sign-in"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let helper = helped.helper_after_sending(&talk, &[(IDLE_SETTING, LONG_IDLE)], FIRST);
    let url = talk.cloud().url.clone();

    let signed_in = helped.run(&[SIGN_IN, SERVER, &url]);

    assert!(signed_in.success, "{signed_in:?}");
    helped.await_exit(helper, EXIT_LIMIT);
    helped.sent(&talk, SECOND);
    assert_ne!(
        helped.one_helper(),
        helper,
        "the stopped helper still serves"
    );
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// With a helper of a newer version running, Ada's `skald sign-in` of the copy's version fails,
/// naming both versions, and the helper keeps running.
#[test]
fn sign_in_beside_a_newer_helper_fails_naming_both_versions() {
    let talk = Talk::new(
        &[4_146_075_001, 4_146_075_002],
        &["ada-signs-in-too-old", "bob-waits-for-sign-in"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let newer = newer();
    let helper = helped.helper_after_sending(&talk, &[(VERSION_SETTING, newer.as_str())], FIRST);
    let url = talk.cloud().url.clone();

    let refused = helped.run(&[SIGN_IN, SERVER, &url]);

    assert!(!refused.success, "{refused:?}");
    for version in [VERSION, newer.as_str()] {
        assert!(
            times(&refused.shown(), version) > 0,
            "{version}: {refused:?}"
        );
    }
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
}
