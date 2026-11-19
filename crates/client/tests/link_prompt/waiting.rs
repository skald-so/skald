//! A waiting `skald recv --wait` shows the device that starts waiting for linking as it shows a
//! warning: the person's then exits; an agent's gets the short line and keeps waiting.

use crate::agents::CLAUDE;
use crate::kit::{RECV, received};
use crate::machine::Run;
use crate::prompt::{LAPTOP, assert_agent_prompted, assert_prompted, waiting_device};
use crate::waits::{WAIT_FLAG, Waits, assert_keeps_waiting, await_connections};
use crate::world::AGENT;

/// Ada's `skald recv` shows what came so far; then `start` starts her `skald recv --wait`, which
/// connects and keeps waiting. Gives it.
#[track_caller]
fn connected_wait(waits: &Waits, start: impl FnOnce() -> Run) -> Run {
    let talk = &waits.agents.talk;
    received(&talk.ada.machine);
    let since = talk.cloud().relay.mark();
    let mut wait = start();
    await_connections(talk, &mut wait, (since, 1));
    assert_keeps_waiting(&mut wait);
    wait
}

/// Ada's `skald recv --wait`, which `start` starts, connects and keeps waiting. Her laptop starts
/// waiting for linking: the wait says once that the server reports it, and exits.
#[track_caller]
fn shows_the_laptop_and_exits(waits: &Waits, start: impl FnOnce() -> Run) {
    let wait = connected_wait(waits, start);

    let _new = waiting_device(&waits.agents.talk.world, &waits.agents.talk.ada, LAPTOP);

    let shown = wait.finish();
    assert!(shown.success, "{shown:?}");
    assert_prompted(&shown.shown(), LAPTOP);
}

/// Ada's `skald recv --wait` in one-shot mode waits on its own connection. Her laptop starts
/// waiting for linking: the wait says once that the server reports it, and exits.
#[test]
fn one_shot_wait_shows_a_device_that_starts_waiting_and_exits() {
    let waits = Waits::new(
        [4_178_201_001, 4_178_201_002],
        ["ada-waits-one-shot-for-a-laptop", "bob-quiet-in-acme"],
    );
    let machine = &waits.agents.talk.ada.machine;
    shows_the_laptop_and_exits(&waits, || machine.start(&[RECV, WAIT_FLAG]));
}

/// Ada's `skald recv --wait` waits through a helper. Her laptop starts waiting for linking: the
/// wait says once that the server reports it, and exits.
#[test]
fn wait_through_a_helper_shows_a_device_that_starts_waiting_and_exits() {
    let waits = Waits::new(
        [4_178_202_001, 4_178_202_002],
        ["ada-waits-helped-for-a-laptop", "bob-quiet-in-acme-too"],
    );
    shows_the_laptop_and_exits(&waits, || waits.helped.start(&[], &[RECV, WAIT_FLAG]));
}

/// Ada's laptop waits for linking. claude's `skald recv --wait` in `~/work/acme`, through a helper,
/// shows the short line once: the server's unverified report of `ada-laptop`, `skald link`, no ID.
#[test]
fn agent_s_wait_gets_the_short_line() {
    let waits = Waits::new(
        [4_178_203_001, 4_178_203_002],
        ["ada-agent-waits-with-a-laptop", "bob-quiet-for-claude"],
    );
    waits.agents.run_as(CLAUDE);
    let _new = waiting_device(&waits.agents.talk.world, &waits.agents.talk.ada, LAPTOP);

    let mut session = waits.session(CLAUDE);
    session.wait_to_show(&[LAPTOP]);

    let shown = session.kill();
    assert_agent_prompted(&shown.shown(), LAPTOP);
}

/// claude's `skald recv --wait` in `~/work/acme`, which `start` starts, connects and keeps
/// waiting. Ada's laptop starts waiting for linking: the wait shows the short line and keeps
/// waiting, for a message.
#[track_caller]
fn agent_s_wait_shows_the_laptop_and_keeps_waiting(waits: &Waits, start: impl FnOnce() -> Run) {
    waits.agents.run_as(CLAUDE);
    let mut wait = connected_wait(waits, start);

    let _new = waiting_device(&waits.agents.talk.world, &waits.agents.talk.ada, LAPTOP);

    wait.wait_to_show(&[LAPTOP]);
    assert_keeps_waiting(&mut wait);
    let shown = wait.kill();
    assert_agent_prompted(&shown.shown(), LAPTOP);
}

/// claude's `skald recv --wait` in one-shot mode waits on its own connection. Ada's laptop starts
/// waiting for linking: the wait shows the short line and keeps waiting.
#[test]
fn agent_s_one_shot_wait_keeps_waiting_after_the_line() {
    let waits = Waits::new(
        [4_178_204_001, 4_178_204_002],
        [
            "ada-agent-waits-one-shot-on",
            "bob-quiet-for-the-agent-wait",
        ],
    );
    let (machine, folder) = (&waits.agents.talk.ada.machine, &waits.agents.folder);
    agent_s_wait_shows_the_laptop_and_keeps_waiting(&waits, || {
        machine.start_in(folder, &[AGENT, CLAUDE, RECV, WAIT_FLAG])
    });
}

/// claude's `skald recv --wait` waits through a helper. Ada's laptop starts waiting for linking:
/// the wait shows the short line and keeps waiting.
#[test]
fn agent_s_wait_through_a_helper_keeps_waiting_after_the_line() {
    let waits = Waits::new(
        [4_178_205_001, 4_178_205_002],
        ["ada-agent-waits-helped-on", "bob-quiet-for-the-helped-wait"],
    );
    agent_s_wait_shows_the_laptop_and_keeps_waiting(&waits, || waits.session(CLAUDE));
}
