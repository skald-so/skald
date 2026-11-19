//! Each command of Ada's first machine that reaches the server, as the person and as an agent,
//! one-shot and through a helper, names her waiting laptop once; a command that makes no request,
//! her laptop itself and another account's machine never do.

use skald_proto::ChannelId;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{send_args, strs};
use crate::kit::RECV;
use crate::machine::{DEVICES, Finished};
use crate::prompt::{
    LAPTOP, UNVERIFIED, assert_agent_prompted, assert_not_prompted, assert_prompted, waiting_device,
};
use crate::reading::HISTORY;
use crate::talk::CAROL_HOST;
use crate::waits::Waits;
use crate::world::{AGENT, RULES, run};

/// The commands that reach the server: `skald recv`, `skald send` of a text of its own to the
/// talk's channel, `skald devices` and `skald history` of that channel. `tag` sets the text apart.
fn commands(channel: ChannelId, tag: &str) -> Vec<Vec<String>> {
    let channel_arg = channel.to_string();
    let text = format!("link prompt test: sent {tag} while a laptop waits");
    vec![
        vec![RECV.to_owned()],
        send_args(channel, &text),
        vec![DEVICES.to_owned()],
        vec![HISTORY.to_owned(), channel_arg],
    ]
}

/// `args` with `--agent claude` before them.
fn as_claude(args: &[String]) -> Vec<String> {
    let mut all = vec![AGENT.to_owned(), CLAUDE.to_owned()];
    all.extend_from_slice(args);
    all
}

/// Fails unless `finished` succeeded; `what` names the run.
#[track_caller]
fn succeeded(finished: Finished, what: &[String]) -> Finished {
    assert!(finished.success, "{what:?}: {finished:?}");
    finished
}

/// Ada's laptop waits for linking. On her first machine in one-shot mode, `skald recv`,
/// `skald send`, `skald devices` and `skald history`, as the person and as claude in `~/work/acme`,
/// each say once that the server reports, unverified, the `client` device `ada-laptop` waiting and
/// that `skald link` links it; claude's line holds no ID. Her `skald rules`, which makes no request,
/// says nothing of it; nor do Carol's `skald devices` and `skald recv` on her own account, nor the
/// laptop's own sign-in.
#[test]
fn every_one_shot_command_names_the_waiting_device_once() {
    let agents = Agents::new(
        [4_178_101_001, 4_178_101_002],
        ["ada-one-shot-sees-her-laptop-wait", "bob-in-acme-one-shot"],
    );
    let talk = &agents.talk;
    let carol = (talk.world).sign_up(4_178_101_003, "carol-sees-no-laptop-of-ada", CAROL_HOST);
    let mut new = waiting_device(&talk.world, &talk.ada, LAPTOP);
    let machine = &talk.ada.machine;

    let by_claude = commands(talk.channel, "by claude, one-shot");
    for (args, claude) in commands(talk.channel, "by ada, one-shot")
        .iter()
        .zip(&by_claude)
    {
        let person = succeeded(machine.start(&strs(args)).finish(), args);
        assert_prompted(&person.shown(), LAPTOP);
        let agent_args = as_claude(claude);
        let agent = machine
            .start_in(&agents.folder, &strs(&agent_args))
            .finish();
        let agent = succeeded(agent, &agent_args);
        assert_agent_prompted(&agent.shown(), LAPTOP);
    }

    let rules = run(machine, &[RULES]);
    assert!(rules.success, "{rules:?}");
    assert_not_prompted(&rules.shown(), LAPTOP);
    for args in [[DEVICES], [RECV]] {
        let other = run(&carol.machine, &args);
        assert!(other.success, "{args:?}: {other:?}");
        assert_not_prompted(&other.shown(), LAPTOP);
    }
    let itself = new.shown();
    assert!(!itself.to_lowercase().contains(UNVERIFIED), "{itself}");
}

/// Ada's laptop waits for linking. On her first machine, through a helper, `skald recv`,
/// `skald send`, `skald devices` and `skald history`, as the person and as claude in `~/work/acme`,
/// each say once that the server reports, unverified, the `client` device `ada-laptop` waiting and
/// that `skald link` links it; claude's line holds no ID. One helper ran them.
#[test]
fn every_command_through_a_helper_names_the_waiting_device_once() {
    let waits = Waits::new(
        [4_178_102_001, 4_178_102_002],
        ["ada-helped-sees-her-laptop-wait", "bob-in-acme-helped"],
    );
    let talk = &waits.agents.talk;
    let _new = waiting_device(&talk.world, &talk.ada, LAPTOP);

    let by_claude = commands(talk.channel, "by claude, through a helper");
    for (args, claude) in
        (commands(talk.channel, "by ada, through a helper").iter()).zip(&by_claude)
    {
        let person = succeeded(waits.helped.run(&strs(args)), args);
        assert_prompted(&person.shown(), LAPTOP);
        let agent_args = as_claude(claude);
        let agent = waits
            .helped
            .run_in(&waits.agents.folder, &strs(&agent_args));
        let agent = succeeded(agent, &agent_args);
        assert_agent_prompted(&agent.shown(), LAPTOP);
    }
    waits.helped.one_helper();
}
