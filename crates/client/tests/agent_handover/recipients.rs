//! Which agents a message goes to: without `to`, each agent of the device in the channel's space;
//! with `to`, only that agent.

use crate::kit::create_space;
use crate::talk::Line;
use crate::world::{ADD_RULE, GLOBEX, dir_of, path, run};
use crate::{Agents, CLAUDE, CODEX, assert_handed_once, assert_not_handed, run_as_in};

/// An agent of Ada's device in another space.
const GEMINI: &str = "gemini-r5c8vn";

/// The texts Bob's device sends; none is anywhere else.
const TO_ALL: &str = "the staging database is back online";
const TO_CODEX: &str = "codex, please rerun the migration job";

/// Ada's device has run as claude and codex in `~/work/acme` and as gemini in `~/work/globex`,
/// bound to `globex`. Bob's device sends to `acme`'s channel a message without `to`, then one to
/// codex. claude's next run shows the first, codex's both, each once with its `msg_id`; gemini's
/// shows neither. The next run of each shows nothing again.
#[test]
fn each_agent_in_the_channel_gets_a_message_without_to_and_only_the_to_agent_one_with_it() {
    let mut agents = Agents::new(
        [4_147_071_001, 4_147_071_002],
        ["ada-hands-to-agents", "bob-writes-to-agents"],
    );
    create_space(agents.talk.cloud(), &agents.talk.ada, GLOBEX);
    let other = dir_of(&agents.talk.ada.machine, "work/globex");
    let bound = run(&agents.talk.ada.machine, &[ADD_RULE, path(&other), GLOBEX]);
    assert!(bound.success, "not bound: {bound:?}");
    agents.run_as(CLAUDE);
    agents.run_as(CODEX);
    run_as_in(&agents.talk.ada.machine, &other, GEMINI);
    let bob = agents.bob();
    let to_all = agents.bob_sends(&Line::new(&bob, 1, TO_ALL));
    let mut line = Line::new(&bob, 2, TO_CODEX);
    line.to = Some(agents.address(CODEX));
    let to_codex = agents.bob_sends(&line);

    let claude = agents.run_as(CLAUDE).shown();
    let codex = agents.run_as(CODEX).shown();
    let elsewhere = run_as_in(&agents.talk.ada.machine, &other, GEMINI).shown();

    assert_handed_once(&claude, TO_ALL, to_all);
    assert_not_handed(&claude, TO_CODEX);
    assert_handed_once(&codex, TO_ALL, to_all);
    assert_handed_once(&codex, TO_CODEX, to_codex);
    for text in [TO_ALL, TO_CODEX] {
        assert_not_handed(&elsewhere, text);
    }
    for again in [agents.run_as(CLAUDE).shown(), agents.run_as(CODEX).shown()] {
        for text in [TO_ALL, TO_CODEX] {
            assert_not_handed(&again, text);
        }
    }
}
