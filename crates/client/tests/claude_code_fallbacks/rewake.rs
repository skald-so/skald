//! The `asyncRewake` hook on `SessionStart` and `Stop`: it waits for the agent's messages up to its
//! timeout, gives them and starts a turn, `own` text as its line and a message below `own` only as
//! the signal, within the hooks' limit; with none, it ends at its timeout printing nothing, and in
//! a folder in no space at once.

use std::fs;
use std::thread;
use std::time::Duration;

use crate::fallbacks::{Bare, late_reply_confirms_once, long_text, rewake_of, session_start, woke};
use crate::hooks::{Bob, Event, LIMIT, assert_cut, assert_own_line, assert_signal};
use crate::solo::{API, WEB};

/// How long the hook waits in a test that gives it a message: well beyond what the test takes.
const WAITS: u64 = 60;

/// What api says to web; it is nowhere else.
const WAITING_SAYS: &str = "rewake test: api asked web before the session started";

/// api sends web a message. web's `asyncRewake` hook starts on `SessionStart`: it exits with 2,
/// which starts a turn, long before its timeout, its standard error the message's line from api at
/// `own`.
#[test]
fn a_waiting_own_message_wakes_the_session_as_its_line() {
    let ada = Bare::new(4_919_511_001, "ada-rewake-waiting-message");
    let msg_id = ada.solo.api_sends_web(WAITING_SAYS);

    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &[]);

    let (hooked, ran) = rewake.finished();
    assert!(
        ran < Duration::from_secs(WAITS / 2),
        "ran {ran:?}: {hooked:?}"
    );
    assert_own_line(&woke(&hooked), WAITING_SAYS, msg_id, &ada.solo.short(API));
}

/// What api says to web; it is nowhere else.
const LATER_SAYS: &str = "rewake test: api asks web while the session is idle";

/// web's `asyncRewake` hook starts on `Stop`, as a turn ends; two seconds later it still runs. api
/// sends web a message: the hook exits with 2, its standard error the message's line from api at
/// `own`.
#[test]
fn an_own_message_that_comes_while_it_waits_wakes_the_session() {
    let ada = Bare::new(4_919_512_001, "ada-rewake-later-message");
    let dir = ada.solo.dir(WEB);
    let stop = serde_json::from_str(&Event::Stop.input(&dir)).expect("an input is JSON");
    let mut rewake = ada.rewake(WEB, &stop, WAITS, &[]);
    thread::sleep(Duration::from_secs(2));
    rewake.assert_running("with nothing waiting");

    let msg_id = ada.solo.api_sends_web(LATER_SAYS);

    let (hooked, _) = rewake.finished();
    assert_own_line(&woke(&hooked), LATER_SAYS, msg_id, &ada.solo.short(API));
}

/// What api says to web; it is nowhere else.
const LATE_SAYS: &str = "rewake test: api asks web before its reply is written";

/// web's `asyncRewake` hook starts on `Stop`; api sends web a message, which the hook gives. Its
/// output, as Claude Code puts it, ends web's session file at the next `Stop`, before bookkeeping
/// lines, and the reply comes later ([`late_reply_confirms_once`]).
#[test]
fn one_stop_confirms_an_own_message_of_the_hook_whose_reply_follows_bookkeeping_lines() {
    let ada = Bare::new(4_919_517_001, "ada-rewake-bookkeeping");
    let dir = ada.solo.dir(WEB);
    let stop = serde_json::from_str(&Event::Stop.input(&dir)).expect("an input is JSON");
    let mut rewake = ada.rewake(WEB, &stop, WAITS, &[]);
    let msg_id = ada.solo.api_sends_web(LATE_SAYS);
    let (hooked, _) = rewake.finished();
    let output = woke(&hooked);
    assert_own_line(&output, LATE_SAYS, msg_id, &ada.solo.short(API));

    let delivery = format!(
        "<task-notification>\n<summary>Stop hook feedback</summary>\n</task-notification>\n\
         <system-reminder>\nStop hook blocking error from command \"Stop\": {}\n</system-reminder>",
        output.trim()
    );
    late_reply_confirms_once(&ada, &delivery, msg_id);
}

/// What review says to web; it is nowhere else.
const PEER_SAYS: &str = "rewake test: review asks web for the coverage report";

/// Bob joins `clienta`; web's `asyncRewake` hook starts on `SessionStart`. review sends web a
/// message from Bob's machine: the hook exits with 2, its standard error the signal alone, with
/// neither review's text, Bob, his device nor the message's reference.
#[test]
fn a_peer_s_message_wakes_the_session_with_the_signal_alone() {
    let ada = Bare::new(4_919_513_001, "ada-rewake-peer-message");
    let bob = Bob::join(&ada.solo, 4_919_513_002, "bob-rewake-peer-message");
    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &[]);

    let msg_id = bob.sends(&ada.solo, WEB, PEER_SAYS);

    let (hooked, _) = rewake.finished();
    assert_signal(&woke(&hooked), msg_id, PEER_SAYS, &bob);
}

/// How long the hook waits in a test that gives it nothing.
const SHORT: u64 = 3;

/// Nothing waits for web; its `asyncRewake` hook starts on `SessionStart`, waiting [`SHORT`]
/// seconds: it exits with success once they passed, printing nothing.
#[test]
fn with_nothing_waiting_it_ends_at_its_timeout_printing_nothing() {
    let ada = Bare::new(4_919_514_001, "ada-rewake-nothing-waits");
    let mut rewake = ada.rewake(WEB, &session_start(), SHORT, &[]);

    let (hooked, ran) = rewake.finished();
    hooked.assert_silent("with nothing waiting");
    assert!(
        ran >= Duration::from_millis(SHORT * 1_000 - 500),
        "ended after {ran:?}, before its timeout"
    );
}

/// `~/work/elsewhere/web` is in no space: an approved `.skald.toml` binds it to a space Ada's
/// device is not in. web's `asyncRewake` hook starts on `SessionStart` there, waiting [`WAITS`]
/// seconds: it exits with success at once, printing nothing, and no helper runs.
#[test]
fn in_a_folder_in_no_space_it_ends_at_once_printing_nothing() {
    let ada = Bare::new(4_919_516_001, "ada-rewake-no-space");
    let elsewhere = ["home", "work", "elsewhere", WEB].iter();
    let machine = ada.solo.machine();
    let elsewhere = machine
        .root()
        .join(elsewhere.collect::<std::path::PathBuf>());
    fs::create_dir_all(&elsewhere).expect("make a folder");
    fs::write(elsewhere.join(".skald.toml"), "space = \"nowhere\"\n").expect("bind the folder");
    let folder = elsewhere.to_string_lossy();
    let approved = (machine.start_in(&elsewhere, &["approve-binding", &folder])).finish();
    assert!(approved.success, "not approved: {approved:?}");
    let mut rewake = rewake_of(&ada.helped, &elsewhere, WEB, &session_start(), WAITS, &[]);

    let (hooked, ran) = rewake.finished();
    hooked.assert_silent("in no space");
    assert!(ran < Duration::from_secs(5), "ended after {ran:?}");
    ada.helped.assert_none();
}

/// What starts api's long message; it is nowhere else.
const LONG_HEAD: &str = "rewake test: api's audit for web, 12000 characters:";

/// api sends web a message of 12,000 characters; web's `asyncRewake` hook starts on `SessionStart`:
/// it exits with 2, its standard error at most 10,000 characters, without the whole text, and as
/// its last line one that names `inbox` and the message's `#<ref>`.
#[test]
fn a_message_over_the_limit_wakes_the_session_cut_with_how_to_read_it_whole() {
    let ada = Bare::new(4_919_515_001, "ada-rewake-long-message");
    let long = long_text(LONG_HEAD, 12_000);
    let msg_id = ada.solo.api_sends_web(&long);

    let mut rewake = ada.rewake(WEB, &session_start(), WAITS, &[]);

    let (hooked, _) = rewake.finished();
    let given = woke(&hooked);
    let length = given.trim_end().chars().count();
    assert!(length <= LIMIT, "{length} characters, over the limit");
    assert_cut(&given, &long, msg_id);
    assert!(
        given.contains(&long[..1_000]),
        "not the long message's: {given}"
    );
}
