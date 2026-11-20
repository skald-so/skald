//! Inbound modes with a stateless run (`docs/protocol/delivery.md`, "Device to agent"): no person
//! sets the run's agent's mode, so it takes every message; a member's agent in `refuse` refuses the
//! run's message unless its allowlist admits the agent.

use crate::agents::{CLAUDE, run_as_in};
use crate::fixtures::{CHANNEL, SEND, TO};
use crate::scene::Scene;
use crate::senders::assert_labelled;
use crate::service_kit::BOT;
use crate::world::run;

/// The commands that set an agent's inbound mode and put a sender on its allowlist.
const SET_MODE: &str = "set-inbound-mode";
const ALLOW: &str = "allow-sender";

/// What the agent and Ada say; none is anywhere else.
const REFUSED_TEXT: &str = "stateless inbound test: claude, pause the rollout";
const ALLOWED_TEXT: &str = "stateless inbound test: claude, resume the rollout";
const ADA_SAYS: &str = "stateless inbound test: deploy-bot, roll back staging";

/// With `~/work/acme` on Ada's machine bound to `acme`, where her agent claude has run once, Ada
/// sets claude to `refuse`. The agent's run sends a line `--to` claude and succeeds; claude's run
/// does not get it, and Ada's `skald recv` marks it refused for claude. The agent's next run
/// succeeds. Once Ada puts `ci/deploy-bot` on claude's allowlist, claude's run gets the agent's
/// next line as a `service` line.
#[test]
fn a_member_s_agent_in_refuse_takes_a_run_s_message_only_by_its_allowlist() {
    let scene = Scene::new(
        [3_135_769_001, 3_135_769_002],
        ["ada-refuses-stateless-runs", "bob-sees-stateless-refusals"],
    );
    let machine = &scene.ada().machine;
    let folder = scene.ada_acme();
    let person_runs = |args: &[&str]| {
        let finished = machine.start_in(&folder, args).finish();
        assert!(finished.success, "{args:?}: {finished:?}");
    };
    person_runs(&[SET_MODE, CLAUDE, "refuse"]);
    let to = format!("{}/{CLAUDE}", scene.ada().handle);
    let channel = scene.channel().to_string();
    let agent_sends = |text: &str| {
        let sent = scene.agent.run(&[SEND, CHANNEL, &channel, TO, &to, text]);
        assert!(sent.success, "the agent's send failed: {sent:?}");
    };

    agent_sends(REFUSED_TEXT);

    let read = run_as_in(machine, &folder, CLAUDE);
    assert!(
        !read.stdout.contains(REFUSED_TEXT),
        "{CLAUDE} got it: {read:?}"
    );
    let shown = run(machine, &[crate::kit::RECV]);
    assert!(shown.success, "{shown:?}");
    let mark = format!("refused for its agent {CLAUDE}");
    assert!(shown.stdout.contains(&mark), "not marked: {shown:?}");
    scene.agent.received();

    person_runs(&[ALLOW, CLAUDE, BOT]);
    agent_sends(ALLOWED_TEXT);

    let read = run_as_in(machine, &folder, CLAUDE);
    assert_labelled(
        &read.stdout,
        ALLOWED_TEXT,
        (BOT, BOT, crate::scene::SERVICE),
    );
}

/// Ada sends a line `--to` the agent. The agent's run, whose mode no person set, shows it once.
#[test]
fn a_run_takes_a_message_to_its_agent() {
    let scene = Scene::new(
        [3_135_790_001, 3_135_790_002],
        ["ada-writes-to-stateless-runs", "bob-watches-stateless-runs"],
    );
    let channel = scene.channel().to_string();
    let args = [SEND, CHANNEL, &channel, TO, BOT, ADA_SAYS];
    let sent = run(&scene.ada().machine, &args);
    assert!(sent.success, "Ada's send failed: {sent:?}");

    let got = scene.agent.received();

    scene.assert_agent_shows(&got, ADA_SAYS, scene.ada());
}
