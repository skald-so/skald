//! `skald recv --wait` in one-shot mode: it holds its own connection and takes the local state's
//! lock only to write, so a run beside it neither waits nor loses a change, and it goes on through
//! a helper that starts while it waits.

use std::thread;
use std::time::{Duration, Instant};

use crate::fixtures::SEND;
use crate::gaps::{bob_in, bob_sends};
use crate::helped::{Helped, TIMEOUT_FLAG, bob_reads};
use crate::kit::{RECV, received};
use crate::locked::{LONG, holding, release, start};
use crate::machine::Run;
use crate::talk::{BOB_HOST, Talk, assert_shows_once};
use crate::waits::{WAIT_FLAG, assert_keeps_waiting, await_connections};

/// What Bob's device says and Ada sends; none is anywhere else.
const BOB_SAYS: &str = "one-shot test: the staging deploy is green";
const FIRST: &str = "one-shot test: promoting staging now";
const SECOND: &str = "one-shot test: production is next";

/// The request timeout of a run that must not wait for the lock, in seconds: it fails after it if
/// it does.
const QUICK: &str = "10";

/// How long a test gives the server's push of a message to reach a waiting run.
const PUSHED: Duration = Duration::from_millis(500);

/// Ada's `skald recv` in one-shot mode shows what came so far; then her `skald recv --wait` in
/// one-shot mode starts, with `flags` before the command, and connects. Gives the run, and the time
/// from which the server saw its connection.
#[track_caller]
fn one_shot_wait(talk: &Talk, flags: &[&str]) -> (Run, Instant) {
    received(&talk.ada.machine);
    let since = talk.cloud().relay.mark();
    let mut args = flags.to_vec();
    args.extend([RECV, WAIT_FLAG]);
    let mut waiting = talk.ada.machine.start(&args);
    await_connections(talk, &mut waiting, (since, 1));
    (waiting, since)
}

/// Ada's `skald recv --wait` in one-shot mode connects and keeps waiting while nothing comes. Bob's
/// device sends a message to the channel: the wait shows it once and exits, and Ada's next
/// `skald recv` does not show it again.
#[test]
fn one_shot_wait_gets_a_message_sent_while_it_waits() {
    let mut talk = Talk::new(
        &[4_148_061_001, 4_148_061_002],
        &["ada-waits-one-shot", "bob-writes-while-she-waits"],
        false,
    );
    let (mut waiting, _) = one_shot_wait(&talk, &[]);
    assert_keeps_waiting(&mut waiting);
    let (from_bob, channel, _) = bob_in(&talk);

    bob_sends(&mut talk, 1, BOB_SAYS);

    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    assert_shows_once(&got.shown(), BOB_SAYS, (channel, &from_bob, BOB_HOST));
    let next = received(&talk.ada.machine);
    assert!(!next.shown().contains(BOB_SAYS), "shown again: {next:?}");
}

/// Ada's `skald recv --wait` in one-shot mode waits on its own connection. Beside it, a
/// `skald send` with a request timeout of [`QUICK`] succeeds, and the wait keeps waiting. A second
/// `skald send` then holds the lock while the front holds back its device's `auth`; meanwhile Bob's
/// device sends a message to the channel, which reaches the waiting run. Once the `auth` goes on,
/// the send catches up on Bob's message, shows nothing of it, and succeeds. The wait shows Bob's
/// message once and exits; Ada's next `skald recv` does not show it again, and Bob's device reads
/// Ada's two messages, with the counters 1 and 2.
#[test]
fn send_beside_a_one_shot_wait_never_waits_and_what_it_caught_up_on_is_shown_by_the_wait() {
    let mut talk = Talk::new(
        &[4_148_062_001, 4_148_062_002],
        &["ada-sends-beside-her-wait", "bob-writes-into-catch-up"],
        false,
    );
    let (mut waiting, _) = one_shot_wait(&talk, &[]);
    let channel = talk.channel.to_string();

    let quick = start(
        &talk,
        QUICK.parse().expect("seconds"),
        &[SEND, &channel, FIRST],
    )
    .finish();

    assert!(quick.success, "it waited beside the wait: {quick:?}");
    assert_keeps_waiting(&mut waiting);
    let (holder, _) = holding(&talk, LONG, &[SEND, &channel, SECOND]);
    let (from_bob, channel_id, _) = bob_in(&talk);
    bob_sends(&mut talk, 1, BOB_SAYS);
    thread::sleep(PUSHED);

    let sent = release(&talk, vec![holder]);

    assert!(
        !sent[0].shown().contains(BOB_SAYS),
        "the send shows it: {sent:?}"
    );
    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    assert_shows_once(&got.shown(), BOB_SAYS, (channel_id, &from_bob, BOB_HOST));
    let next = received(&talk.ada.machine);
    assert!(!next.shown().contains(BOB_SAYS), "shown again: {next:?}");
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// Ada's `skald recv --wait` in one-shot mode, with a request timeout of [`QUICK`], waits on its
/// own connection. Her `skald recv` then starts a helper, which connects and holds the local
/// state's lock. Bob's device
/// sends a message to the channel: the wait shows it once and exits, though it could not take the
/// lock to write it itself. The helper keeps running, and Ada's next `skald recv`, through it, does
/// not show the message again.
#[test]
fn one_shot_wait_goes_on_through_a_helper_started_while_it_waits() {
    let mut talk = Talk::new(
        &[4_148_063_001, 4_148_063_002],
        &["ada-gets-a-helper-mid-wait", "bob-writes-to-the-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let (mut waiting, since) = one_shot_wait(&talk, &[TIMEOUT_FLAG, QUICK]);
    helped.received();
    let helper = helped.one_helper();
    await_connections(&talk, &mut waiting, (since, 2));
    assert_keeps_waiting(&mut waiting);
    let (from_bob, channel, _) = bob_in(&talk);

    bob_sends(&mut talk, 1, BOB_SAYS);

    let started = Instant::now();
    let got = waiting.finish();
    assert!(got.success, "{got:?}");
    assert_shows_once(&got.shown(), BOB_SAYS, (channel, &from_bob, BOB_HOST));
    assert!(
        started.elapsed() < Duration::from_secs(QUICK.parse().expect("seconds")),
        "it waited for the lock: {got:?}"
    );
    let next = helped.received();
    assert!(!next.shown().contains(BOB_SAYS), "shown again: {next:?}");
    assert_eq!(
        helped.helpers(),
        [helper],
        "the helper did not keep running"
    );
}
