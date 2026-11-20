//! Beside the sessions and runs above: a one-shot send of a new agent, a revoked device, a waiting
//! `skald recv --wait` and a helper that exits under it.

use skald_proto::AgentState;

use crate::agents::{Agents, CLAUDE};
use crate::fixtures::{ALL, CHANNEL, Issuer, SEND, add_device};
use crate::helped::{Helped, IDLE_SETTING, LONG_IDLE, VERSION_SETTING, newer};
use crate::kit::RECV;
use crate::revocations::{self, signed, store_revocation};
use crate::values::{
    acme_channels, ada_s_body, ada_s_value, addresses, assert_between, await_available,
    await_in_each, body_of, listed, ran_as, run_with, shows, uploads,
};
use crate::world::{AGENT, now};

/// The flag of `skald recv` that waits for a message.
const WAIT_FLAG: &str = "--wait";

/// The command that lists the account's devices: it starts no helper.
const DEVICES: &str = "devices";

/// What claude sends; nothing else holds it.
const TEXT: &str = "presence runs test: claude says hello";

/// Claude, never run before, sends a message one-shot to the channel in `~/work/acme`. Once the
/// run is over, Ada's value in each channel of `acme` lists claude, `offline`, and only claude.
#[test]
fn one_shot_send_of_a_new_agent_uploads_it_listed() {
    let agents = Agents::new(
        [4_731_071_001, 4_731_071_002],
        ["ada-sends-as-claude-once", "bob-sees-claude-listed"],
    );
    let acme = acme_channels(&agents);
    let claude = agents.address(CLAUDE);
    let channel = agents.talk.channel.to_string();

    ran_as(&agents, CLAUDE, &[SEND, ALL, CHANNEL, &channel, TEXT]);

    for at in acme {
        let body = ada_s_body(&agents.talk, at).expect("a value of Ada's");
        assert_eq!(addresses(&body), std::slice::from_ref(&claude), "{body:?}");
        assert_eq!(
            listed(&body, &claude).state,
            AgentState::Offline,
            "{body:?}"
        );
    }
}

/// Another device of Ada's revokes her machine's device. Claude's one-shot `skald recv` in
/// `~/work/acme` then uploads no value in any channel of `acme`.
#[test]
fn revoked_device_uploads_no_value() {
    let agents = Agents::new(
        [4_731_072_001, 4_731_072_002],
        ["ada-revoked-machine", "bob-sees-no-value"],
    );
    let (cloud, ada) = (agents.talk.cloud(), &agents.talk.ada);
    let other = add_device(cloud, ada.account, "ada-old-laptop", &Issuer::Device(ada));
    let revocation = revocations::body(ada.account, ada.device, Some(other.id));
    store_revocation(cloud, &ada.token, &signed(&revocation, &other.signing));
    let since = cloud.relay.mark();

    // Its result does not matter: the machine's device is revoked.
    let ran = run_with(&ada.machine, &agents.folder, &[], &[AGENT, CLAUDE, RECV]);

    for at in acme_channels(&agents) {
        assert_eq!(uploads(&agents.talk, at, since), 0, "in {}: {ran:?}", at.1);
    }
}

/// Through a helper that waits long idle, claude's `skald recv --wait` waits in `~/work/acme`:
/// Ada's value in each channel of `acme` lists claude `available` since it started. The run is
/// killed: while the helper still runs, her values come to list claude `offline`, since it
/// ended.
#[test]
fn waiting_recv_shows_its_agent_available_and_offline_once_it_ended() {
    let mut agents = Agents::new(
        [4_731_073_001, 4_731_073_002],
        ["ada-waits-as-claude", "bob-sees-the-wait"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let started = now();
    let long = [(IDLE_SETTING, LONG_IDLE)];
    let waiting = helped.start_in(&agents.folder, &long, &[AGENT, CLAUDE, RECV, WAIT_FLAG]);

    let (claude, acme) = await_available(&mut agents, CLAUDE);

    for at in acme {
        let body = ada_s_body(&agents.talk, at).expect("a value of Ada's");
        assert_between(listed(&body, &claude).since, (started, now()), "available");
    }
    let ended = now();
    waiting.kill();

    let offline = |body: &_| shows(body, &claude, AgentState::Offline);
    let bodies = await_in_each(&mut agents.talk, &acme, "with claude offline", offline);

    for body in bodies {
        assert_between(listed(&body, &claude).since, (ended, now()), "offline");
    }
    // The helper still runs: no exit of it left claude offline.
    helped.one_helper();
}

/// Through a helper, claude's `skald recv --wait` waits in `~/work/acme`, shown `available`. The
/// person's `skald devices` of a newer version has the helper exit: by the time the run is over,
/// Ada's value in each channel of `acme` lists claude `offline`, since the helper exited.
#[test]
fn helper_exiting_under_a_session_leaves_its_agent_offline() {
    let mut agents = Agents::new(
        [4_731_074_001, 4_731_074_002],
        ["ada-replaces-her-helper", "bob-sees-claude-leave"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let waiting = helped.start_in(&agents.folder, &[], &[AGENT, CLAUDE, RECV, WAIT_FLAG]);
    let (claude, acme) = await_available(&mut agents, CLAUDE);
    let newer = newer();
    let before = now();

    let listed_devices = helped.run_with(&[(VERSION_SETTING, newer.as_str())], &[DEVICES]);

    let after = now();
    for at in acme {
        let value = ada_s_value(&agents.talk, at).expect("a value of Ada's");
        let body = body_of(&agents.talk, &value);
        let status = listed(&body, &claude);
        assert_eq!(
            status.state,
            AgentState::Offline,
            "{body:?}: {listed_devices:?}"
        );
        assert_between(status.since, (before, after), "offline");
    }
    waiting.kill();
}
