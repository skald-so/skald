//! One helper for the commands of a machine: started by the first command that connects, reused
//! by the next ones, gone when idle or signed out; and runs in one-shot mode beside it.

use std::fs;
use std::time::Duration;

use crate::helped::{
    EXIT_LIMIT, Helped, IDLE_SETTING, LONG_IDLE, ONE_SHOT, ONE_SHOT_SETTING, START_TIMEOUT,
    TIMEOUT_FLAG, WEBSOCKET, bob_reads, timeout_of,
};
use crate::kit::RECV;
use crate::machine::SIGN_OUT;
use crate::talk::Talk;
use crate::world::{ACME, ADD_RULE, APPROVE, FOLDER, REMOVE_RULE, RULES, dir_of, path, words};

/// The texts the tests send; none is anywhere else.
const FIRST: &str = "helper test: the nightly build is green";
const SECOND: &str = "helper test: deploying the nightly build";

/// The idle time of the helper whose exit a test waits for, in seconds.
const SHORT_IDLE: u64 = 6;

/// The folder of the rule a test adds and removes, under `~/work`.
const RULE_FOLDER: &str = "acme-rule";

/// The request timeout of a run that would wait for the lock if it did not go through the helper,
/// in seconds: such a run fails after it.
const SHORT_TIMEOUT: &str = "5";

/// The first `skald send` on Ada's machine succeeds and leaves one helper running: its outputs
/// closed when it exited, so the helper is detached from them. A `skald recv` and a second
/// `skald send` succeed through that helper, which still runs; the server saw one WebSocket of
/// Ada's machine for all three. Bob's device reads both messages, with the counters 1 and 2.
#[test]
fn first_send_starts_the_helper_and_later_commands_reuse_its_connection() {
    let mut talk = Talk::new(
        &[4_146_011_001, 4_146_011_002],
        &["ada-starts-a-helper", "bob-reads-through-helper"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    helped.assert_none();
    let since = talk.cloud().relay.mark();

    let helper = helped.helper_after_sending(&talk, &[], FIRST);
    helped.received();
    helped.sent(&talk, SECOND);
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    assert_eq!(
        talk.cloud().count_sent(since, WEBSOCKET),
        1,
        "not one connection for three commands"
    );
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// With an idle time of [`SHORT_IDLE`], the helper the first `skald send` started exits once idle.
/// The next `skald send` starts a new helper, with a connection of its own, and Bob's device reads
/// both messages.
#[test]
fn helper_exits_when_idle_and_the_next_command_starts_a_new_one() {
    let mut talk = Talk::new(
        &[4_146_012_001, 4_146_012_002],
        &["ada-idles-the-helper", "bob-reads-after-idle"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let seconds = SHORT_IDLE.to_string();
    let [timeout] = timeout_of(START_TIMEOUT);
    let idle = [(IDLE_SETTING, seconds.as_str()), timeout];
    let first = helped.send_with(&talk, &idle, FIRST);
    assert!(first.success, "{first:?}");
    let helper = helped.one_helper();

    helped.await_exit(helper, Duration::from_secs(SHORT_IDLE) + EXIT_LIMIT);

    let since = talk.cloud().relay.mark();
    let second = helped.send_with(&talk, &idle, SECOND);
    assert!(second.success, "{second:?}");
    let next = helped.one_helper();
    assert_ne!(next, helper, "the idle helper still serves");
    assert_eq!(
        talk.cloud().count_sent(since, WEBSOCKET),
        1,
        "the new helper has no connection of its own"
    );
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// With a helper running whose idle time is well above the test's wait for its exit,
/// `skald sign-out` succeeds and the helper exits.
#[test]
fn helper_exits_on_sign_out() {
    let talk = Talk::new(
        &[4_146_013_001, 4_146_013_002],
        &["ada-signs-out-helper", "bob-sees-sign-out"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let first = helped.send_with(&talk, &[(IDLE_SETTING, LONG_IDLE)], FIRST);
    assert!(first.success, "{first:?}");
    let helper = helped.one_helper();

    let signed_out = helped.run(&[SIGN_OUT]);

    assert!(signed_out.success, "{signed_out:?}");
    helped.await_exit(helper, EXIT_LIMIT);
}

/// With the one-shot flag, and then with the one-shot setting, `skald send` succeeds and leaves no
/// helper; each run had a connection of its own. Bob's device reads both messages.
#[test]
fn one_shot_flag_or_setting_starts_no_helper() {
    let mut talk = Talk::new(
        &[4_146_014_001, 4_146_014_002],
        &["ada-runs-one-shot", "bob-reads-one-shot"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let channel = talk.channel.to_string();
    let since = talk.cloud().relay.mark();

    let flagged = helped.run(&[
        ONE_SHOT,
        crate::fixtures::SEND,
        "--all",
        "--channel",
        &channel,
        FIRST,
    ]);

    assert!(flagged.success, "{flagged:?}");
    helped.assert_none();

    let set = helped.send_with(&talk, &[(ONE_SHOT_SETTING, "1")], SECOND);

    assert!(set.success, "{set:?}");
    helped.assert_none();
    assert_eq!(
        talk.cloud().count_sent(since, WEBSOCKET),
        2,
        "not a connection per one-shot run"
    );
    bob_reads(&mut talk, &[FIRST, SECOND]);
}

/// With a helper running, which holds the local state's lock, runs with a request timeout of
/// [`SHORT_TIMEOUT`] go through it and succeed without waiting for the lock: `skald send` with the
/// one-shot flag, and with the one-shot setting `skald recv`; `skald add-rule` of
/// `~/work/acme-rule` to `acme`, after which `skald rules` lists it; `skald remove-rule`, after which it lists none;
/// and `skald approve-binding` of `~/work/approved`, whose `.skald.toml` names `acme`, after which
/// `skald folder` shows that folder in `acme`, bound by its `.skald.toml`. The same helper still
/// runs, no command opened a connection of its own, and Bob's device reads both messages.
#[test]
fn one_shot_runs_and_folder_bindings_go_through_a_running_helper_without_waiting_for_the_lock() {
    let mut talk = Talk::new(
        &[4_146_015_001, 4_146_015_002],
        &["ada-binds-through-helper", "bob-reads-bound-sends"],
        false,
    );
    let helped = Helped::new(&talk.ada.machine);
    let helper = helped.helper_after_sending(&talk, &[], FIRST);
    let rule = dir_of(&talk.ada.machine, &format!("work/{RULE_FOLDER}"));
    let approved = dir_of(&talk.ada.machine, "work/approved");
    fs::write(
        approved.join(".skald.toml"),
        format!("space = \"{ACME}\"\n"),
    )
    .expect("write a .skald.toml");
    let channel = talk.channel.to_string();
    let since = talk.cloud().relay.mark();
    let quick = |settings: &[(&str, &str)], args: &[&str]| {
        let mut all = vec![TIMEOUT_FLAG, SHORT_TIMEOUT];
        all.extend_from_slice(args);
        let finished = helped.run_with(settings, &all);
        assert!(
            finished.success,
            "{args:?} did not go through the helper: {finished:?}"
        );
        finished
    };

    quick(
        &[],
        &[
            ONE_SHOT,
            crate::fixtures::SEND,
            "--all",
            "--channel",
            &channel,
            SECOND,
        ],
    );
    quick(&[(ONE_SHOT_SETTING, "1")], &[RECV]);
    quick(&[], &[ADD_RULE, path(&rule), ACME]);
    let listed = helped.run(&[RULES]);
    quick(&[], &[REMOVE_RULE, path(&rule)]);
    let emptied = helped.run(&[RULES]);
    quick(&[], &[APPROVE, path(&approved)]);
    let shown = helped.run(&[FOLDER, path(&approved)]);

    assert!(
        listed
            .shown()
            .lines()
            .any(|line| line.contains(RULE_FOLDER) && words(line).contains(&ACME)),
        "the rule is not listed: {listed:?}"
    );
    assert!(
        !emptied.shown().contains(RULE_FOLDER),
        "the rule is still listed: {emptied:?}"
    );
    let shown_text = shown.shown();
    assert!(
        shown_text.contains(ACME) && shown_text.contains(".skald.toml"),
        "the approved folder is not in {ACME} by its .skald.toml: {shown:?}"
    );
    assert_eq!(helped.helpers(), [helper], "not the same helper");
    assert_eq!(
        talk.cloud().count_sent(since, WEBSOCKET),
        0,
        "a run beside the helper connected on its own"
    );
    bob_reads(&mut talk, &[FIRST, SECOND]);
}
