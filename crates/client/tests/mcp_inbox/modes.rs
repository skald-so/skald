//! Tool calls one-shot and through the helper, as a command runs (`helper/main.rs`), and calls
//! after the binary was replaced by a newer one while the server runs.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::agents::Agents;
use crate::common::CLIENT as SKALD;
use crate::helped::{
    EXIT_LIMIT, Helped, ONE_SHOT, ONE_SHOT_SETTING, VERSION_SETTING, older, times,
};
use crate::lines::assert_handed_line;
use crate::talk::Line;
use crate::{CLIENT, MCP, Session, assert_nothing_waiting};

/// What Bob's device says; none is anywhere else.
const MODES: [&str; 3] = [
    "mode test: read with the one-shot flag",
    "mode test: read with the one-shot setting",
    "mode test: read through the helper",
];
const REPLACED: [&str; 3] = [
    "replace test: before the update",
    "replace test: kept while the update ran",
    "replace test: after the update",
];
const UPDATED: [&str; 2] = [
    "replace test: read with no helper running",
    "replace test: read one-shot",
];

/// The settings and the arguments before `mcp` of a session.
type Mode<'a> = (&'a [(&'a str, &'a str)], &'a [&'a str]);

/// Starts `skald <args> mcp` of `helped`'s copy in `dir`, with `settings`, and initializes it as
/// `claude-code`.
fn helped_session(helped: &Helped, dir: &Path, (settings, args): Mode) -> Session {
    let mut all = args.to_vec();
    all.push(MCP);
    Session::start(&mut helped.command_in(dir, settings, &all), CLIENT)
}

/// As [`helped_session`] in `dir`, with `args`, the session acting as an older version than the
/// copy on disk.
fn older_session(helped: &Helped, dir: &Path, args: &[&str]) -> Session {
    let older = older();
    helped_session(helped, dir, (&[(VERSION_SETTING, older.as_str())], args))
}

/// The processes of `helped`'s copy beside `session`'s server: its helpers.
fn helpers_beside(helped: &Helped, session: &Session) -> Vec<u32> {
    let server = session.id();
    (helped.helpers().into_iter())
        .filter(|pid| *pid != server)
        .collect()
}

/// The one helper of `helped`'s copy beside `session`'s server, once the other processes of the
/// copy, such as one that ran a call, have exited; fails unless there is one within [`EXIT_LIMIT`].
#[track_caller]
fn one_helper_beside(helped: &Helped, session: &Session) -> u32 {
    let deadline = Instant::now() + EXIT_LIMIT;
    loop {
        match helpers_beside(helped, session).as_slice() {
            [one] => return *one,
            other => assert!(Instant::now() < deadline, "not one helper: {other:?}"),
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Sessions of `claude-code` in `~/work/acme`, of Ada's machine's own copy of `skald`, each in
/// turn: with `--one-shot`, then with `SKALD_ONE_SHOT=1`, then with neither. Bob's device sends a
/// message to the channel for each: each session's `inbox` shows it once, by its reference; beside
/// the one-shot sessions no helper runs, and beside the last one helper runs.
#[test]
fn inbox_runs_one_shot_with_the_flag_or_the_setting_and_through_a_helper_otherwise() {
    let mut agents = Agents::new(
        [4_627_041_001, 4_627_041_002],
        ["ada-tries-each-mode", "bob-writes-in-each-mode"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let one_shot = [(ONE_SHOT_SETTING, "1")];
    let modes: [Mode; 3] = [(&[], &[ONE_SHOT]), (&one_shot, &[]), (&[], &[])];
    let bob = agents.bob();
    for ((mode, text), counter) in modes.into_iter().zip(MODES).zip(1..) {
        let mut session = helped_session(&helped, &agents.folder, mode);
        let id = agents.bob_sends(&Line::new(&bob, counter, text));

        let shown = session.inbox();

        assert_handed_line(&shown, text, id);
        let helpers = helpers_beside(&helped, &session);
        let expected = usize::from(counter == 3);
        assert_eq!(helpers.len(), expected, "{text}: helpers {helpers:?}");
    }
}

/// A session of `claude-code` in `~/work/acme`, of Ada's machine's own copy of `skald`, acts as an
/// older version than the copy on disk: its binary was replaced by a newer one. Bob's device sends a
/// message to the channel, and the session's `inbox` shows it. Bob's device sends another, which the
/// person's `skald recv` of the copy's version takes in, starting a helper of that version; then a
/// third. The session's next `inbox` is served, not refused: it shows the second and the third once
/// each, by their references, and not the first; the newer helper still runs. Its `inbox` after
/// that gives one short line.
#[test]
fn call_after_the_binary_is_replaced_by_a_newer_one_is_served_and_loses_nothing() {
    let mut agents = Agents::new(
        [4_627_042_001, 4_627_042_002],
        [
            "ada-updates-under-her-session",
            "bob-writes-across-the-update",
        ],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let mut session = older_session(&helped, &agents.folder, &[]);
    let bob = agents.bob();
    let first = agents.bob_sends(&Line::new(&bob, 1, REPLACED[0]));
    assert_handed_line(&session.inbox(), REPLACED[0], first);
    let second = agents.bob_sends(&Line::new(&bob, 2, REPLACED[1]));
    let person = helped.received().shown();
    assert!(person.contains(REPLACED[1]), "{person}");
    let newer = helpers_beside(&helped, &session);
    assert_eq!(newer.len(), 1, "not one helper: {newer:?}");
    let third = agents.bob_sends(&Line::new(&bob, 3, REPLACED[2]));

    let shown = session.inbox();

    assert_handed_line(&shown, REPLACED[1], second);
    assert_handed_line(&shown, REPLACED[2], third);
    assert_eq!(times(&shown, REPLACED[0]), 0, "shown again: {shown}");
    let beside = helpers_beside(&helped, &session);
    assert!(
        beside.contains(&newer[0]),
        "the newer helper {newer:?} no longer runs: {beside:?}"
    );
    assert_nothing_waiting(&session.inbox());
}

/// A session of `claude-code` in `~/work/acme`, of Ada's machine's own copy of `skald`, acts as an
/// older version; the helper its start started is killed, so none runs. The copy is replaced by a
/// newer one, as `skald update` replaces it. Bob's device sends a message to the channel, and the
/// session's `inbox` shows it once, by its reference: the newer copy serves it, so the one helper
/// it starts is of the newer version, which the person's `skald recv` of the copy goes through.
#[test]
fn call_after_the_binary_is_replaced_with_no_helper_running_is_served_by_the_newer_one() {
    let mut agents = Agents::new(
        [4_627_043_001, 4_627_043_002],
        ["ada-updates-with-no-helper", "bob-writes-past-the-update"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let mut session = older_session(&helped, &agents.folder, &[]);
    for pid in helpers_beside(&helped, &session) {
        helped.kill(pid);
    }
    helped.replace(Path::new(SKALD));
    let bob = agents.bob();
    let id = agents.bob_sends(&Line::new(&bob, 1, UPDATED[0]));

    let shown = session.inbox();

    assert_handed_line(&shown, UPDATED[0], id);
    let newer = one_helper_beside(&helped, &session);
    helped.received();
    let after = helpers_beside(&helped, &session);
    assert_eq!(after, [newer], "the helper was not of the newer version");
}

/// A session of `claude-code` in `~/work/acme`, of Ada's machine's own copy of `skald`, runs
/// with `--one-shot` and acts as an older version. The copy is replaced by a newer one that
/// records each of its runs' arguments. Bob's device sends a message to the channel, and the
/// session's `inbox` shows it once, by its reference: the newer copy ran it, one-shot.
#[cfg(unix)]
#[test]
fn one_shot_call_after_the_binary_is_replaced_is_run_by_the_newer_one() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    let mut agents = Agents::new(
        [4_627_044_001, 4_627_044_002],
        ["ada-updates-one-shot", "bob-writes-past-a-one-shot-update"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let mut session = older_session(&helped, &agents.folder, &[ONE_SHOT]);
    let bin = helped.exe().parent().expect("the copy is in bin/");
    let (real, runs, recording) = (bin.join("real"), bin.join("runs"), bin.join("recording"));
    fs::copy(SKALD, &real).expect("copy skald");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        runs.display(),
        real.display()
    );
    fs::write(&recording, script).expect("write the recording copy");
    fs::set_permissions(&recording, fs::Permissions::from_mode(0o755)).expect("make it runnable");
    helped.replace(&recording);
    let bob = agents.bob();
    let id = agents.bob_sends(&Line::new(&bob, 1, UPDATED[1]));

    let shown = session.inbox();

    assert_handed_line(&shown, UPDATED[1], id);
    let runs = fs::read_to_string(&runs).unwrap_or_default();
    let ran = runs.lines().any(|run| {
        let args: Vec<&str> = run.split_whitespace().collect();
        args.contains(&MCP) && args.contains(&ONE_SHOT)
    });
    assert!(
        ran,
        "the newer copy did not run the call one-shot: {runs:?}"
    );
}
