//! Only the person at a terminal sees key changes: from a pipe and as an agent the command refuses
//! and they wait; a run killed after showing them and before counting them seen leaves them
//! waiting.

use crate::agents::{Agents, CLAUDE};
use crate::machine::Finished;
use crate::terminal::Terminal;
use crate::world::{AGENT, RULES};
use crate::{
    KEY_CHANGES, asking, assert_nothing_to_show, assert_run_says_none, assert_run_says_waiting,
    bobs_names, bobs_two_masters, names_one_of, seen_at_terminal,
};

/// Fails unless `refused`, the run `what` names, failed and showed none of `names`.
#[track_caller]
fn assert_refused(what: &str, (success, text): (bool, &str), names: &[String]) {
    assert!(!success, "{what} succeeded: {text}");
    assert!(
        !names_one_of(text, names),
        "{what} showed a key change: {text}"
    );
}

/// Fails unless `finished`, the run `what` names, failed and showed none of `names`.
#[track_caller]
fn assert_run_refused(what: &str, finished: &Finished, names: &[String]) {
    assert_refused(what, (finished.success, &finished.shown()), names);
}

/// One key change waits on Ada's machine. `skald key-changes` through a pipe, and as claude in
/// `~/work/acme` through a pipe and at a terminal, each fail and show nothing of it. The key change
/// still waits: her `skald rules` prints the line, and her `skald key-changes` at a terminal shows
/// it and counts it seen.
#[test]
fn from_a_pipe_or_as_an_agent_the_key_changes_are_refused_and_wait() {
    let agents = Agents::new(
        [4_188_801_001, 4_188_801_002],
        ["ada-refuses-to-show-in-a-pipe", "bob-two-masters-refused"],
    );
    bobs_two_masters(&agents);
    let (machine, folder) = (&agents.talk.ada.machine, &agents.folder);
    let names = bobs_names(&agents);

    let piped = machine.start(&[KEY_CHANGES]).finish();
    let agent_piped = machine
        .start_in(folder, &[AGENT, CLAUDE, KEY_CHANGES])
        .finish();
    let agent_at_terminal =
        Terminal::start_in(machine, folder, &[AGENT, CLAUDE, KEY_CHANGES]).finish();

    assert_run_refused("skald key-changes through a pipe", &piped, &names);
    assert_run_refused(
        "claude's skald key-changes through a pipe",
        &agent_piped,
        &names,
    );
    let at_terminal = (agent_at_terminal.success, agent_at_terminal.text.as_str());
    assert_refused(
        "claude's skald key-changes at a terminal",
        at_terminal,
        &names,
    );
    assert_run_says_waiting(machine, &[RULES]);
    let shown = seen_at_terminal(machine);
    assert!(
        names_one_of(&shown.text, &names),
        "Bob's key change is not shown: {}",
        shown.text
    );
    assert_run_says_none(machine, &[RULES]);
}

/// One key change waits on Ada's machine. Her `skald key-changes` at a terminal shows it and is
/// killed while it asks to press Enter: her `skald rules` still prints the line, and her next `skald
/// key-changes` at a terminal shows it again and, on Enter, counts it seen; then `skald rules`
/// prints no line and `skald key-changes` has nothing to show.
#[test]
fn killed_before_counting_them_seen_the_key_changes_show_again() {
    let agents = Agents::new(
        [4_188_802_001, 4_188_802_002],
        ["ada-killed-while-reading", "bob-two-masters-killed"],
    );
    bobs_two_masters(&agents);
    let machine = &agents.talk.ada.machine;
    let names = bobs_names(&agents);

    let killed = asking(machine).kill();

    assert!(
        names_one_of(&killed.text, &names),
        "Bob's key change is not shown: {}",
        killed.text
    );
    assert_run_says_waiting(machine, &[RULES]);
    let again = seen_at_terminal(machine);
    assert!(
        names_one_of(&again.text, &names),
        "not shown again: {}",
        again.text
    );
    assert_run_says_none(machine, &[RULES]);
    assert_nothing_to_show(machine, &names);
}
