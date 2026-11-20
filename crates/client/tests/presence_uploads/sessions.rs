//! Sessions through the helper: each agent with a running MCP session `available` in every channel
//! of its space, with its runtime and since when; `offline` once its session ends; every change in
//! every channel of the device together, those of other spaces listing none of the agents; and a
//! device with no agent uploading empty `agents`.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{AgentState, ChannelId, SpaceId};

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::cloud::WAIT;
use crate::helped::{Helped, with_client_b};
use crate::presence::assert_between;
use crate::talk::Talk;
use crate::values::{
    EXPIRES_IN, acme_channels, ada_s_body, ada_s_value, await_in_each, await_value,
    claude_code_session, helped_session, listed, shows, uploads,
};
use crate::world::now;

/// The name the second agent CLI gives itself at initialization, and the runtime it makes.
const CODEX_CLIENT: &str = "Codex CLI";
const CODEX_RUNTIME: &str = "codex-cli";
/// The runtime of `claude-code`.
const CLAUDE_RUNTIME: &str = "claude-code";

/// Waits for up to `limit` until the machines have uploaded as many values in each of `ats` from
/// `since` on, and at least one; fails unless they have. Gives the count.
#[track_caller]
fn await_together(talk: &Talk, ats: &[(SpaceId, ChannelId)], since: Instant) -> usize {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let counts: Vec<usize> = ats.iter().map(|at| uploads(talk, *at, since)).collect();
        if counts[0] > 0 && counts.iter().all(|count| *count == counts[0]) {
            return counts[0];
        }
        assert!(
            Instant::now() < deadline,
            "not as many uploads in each channel: {counts:?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Ada's machine follows `clientb` too ([`with_client_b`]) and runs through a helper. The person's
/// `skald recv` runs on it until her value in each channel of `acme` and in clientb's is of an epoch
/// Bob's device holds: her device has added his to those channels' groups, a Commit that changes
/// their epochs and so uploads in them alone, and uploaded there since. Fails unless it has within
/// [`WAIT`]. Gives the helper and the three channels, clientb's last.
#[track_caller]
fn settled(agents: &mut Agents) -> (Helped, [(SpaceId, ChannelId); 3]) {
    let (clientb, elsewhere) = with_client_b(&mut agents.talk);
    let helped = Helped::new(&agents.talk.ada.machine);
    let acme = acme_channels(agents);
    let ats = [acme[0], acme[1], (clientb.id, elsewhere)];
    let talk = &mut agents.talk;
    let deadline = Instant::now() + WAIT;
    loop {
        helped.received();
        talk.played[0].take_in(&talk.world.cloud);
        let held = |at: &(SpaceId, ChannelId)| {
            ada_s_value(talk, *at).is_some_and(|value| {
                (talk.played[0].mls)
                    .history_key(value.channel, value.epoch)
                    .is_some()
            })
        };
        if ats.iter().all(held) {
            return (helped, ats);
        }
        assert!(
            Instant::now() < deadline,
            "Ada's device has not added Bob's to every group"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Ada's machine follows `acme` and `clientb`; `~/work/acme` is bound to `acme`. Through one
/// helper, claude runs an MCP session of `claude-code` and codex one of `Codex CLI`, both in
/// `~/work/acme`. Ada's value in each channel of `acme`, the default one and the talk's, lists each
/// once, `available` since its session started, with the runtimes `claude-code` and `codex-cli`,
/// and expires 900 seconds after its upload; her value in clientb's channel lists none of them.
/// The machine uploaded as many values in each of the three channels.
#[test]
fn running_sessions_show_their_agents_available_with_runtimes_in_each_channel_of_the_space() {
    let mut agents = Agents::new(
        [4_731_001_001, 4_731_001_002],
        ["ada-runs-two-agent-clis", "bob-reads-her-presence"],
    );
    let (helped, all) = settled(&mut agents);
    let (acme, clientb) = (&all[..2], all[2]);
    let since = agents.talk.cloud().relay.mark();
    let started = now();

    let _claude = claude_code_session(&helped, &agents, CLAUDE);
    let _codex = helped_session(&helped, &agents.folder, &[], (CODEX, CODEX_CLIENT));

    let (claude, codex) = (agents.address(CLAUDE), agents.address(CODEX));
    let bodies = await_in_each(&mut agents.talk, acme, "with both available", |body| {
        shows(body, &claude, AgentState::Available) && shows(body, &codex, AgentState::Available)
    });
    let shown = now();
    for body in &bodies {
        for (address, runtime) in [(&claude, CLAUDE_RUNTIME), (&codex, CODEX_RUNTIME)] {
            let status = listed(body, address);
            let made = status.runtime.as_ref().map(|name| name.to_string());
            assert_eq!(made.as_deref(), Some(runtime), "{status:?}");
            assert_between(status.since, (started, shown), "available");
        }
        assert_eq!(body.agents.len(), 2, "{body:?}");
        assert_between(
            body.expires,
            (started + EXPIRES_IN, shown + EXPIRES_IN),
            "expires",
        );
    }
    await_together(&agents.talk, &all, since);
    let (_, other) = await_value(&mut agents.talk, clientb, "in clientb", |_, _| true);
    assert!(
        other.agents.is_empty(),
        "clientb lists acme's agents: {other:?}"
    );
}

/// Through one helper, claude and codex run MCP sessions in `~/work/acme`, both shown `available`.
/// Claude's session ends. In each channel of `acme`, Ada's value then lists claude `offline` since
/// its session ended and codex still `available` since it was; her value in clientb's channel was
/// uploaded too, as many times as in the others, listing no agent.
#[test]
fn ended_session_shows_its_agent_offline_since_it_ended_in_every_channel_together() {
    let mut agents = Agents::new(
        [4_731_002_001, 4_731_002_002],
        ["ada-closes-one-session", "bob-sees-claude-leave"],
    );
    let (helped, all) = settled(&mut agents);
    let (acme, clientb) = (&all[..2], all[2]);
    let start = agents.talk.cloud().relay.mark();
    let claude_session = claude_code_session(&helped, &agents, CLAUDE);
    let _codex = claude_code_session(&helped, &agents, CODEX);
    let (claude, codex) = (agents.address(CLAUDE), agents.address(CODEX));
    let before = await_in_each(&mut agents.talk, acme, "with both available", |body| {
        shows(body, &claude, AgentState::Available) && shows(body, &codex, AgentState::Available)
    });
    // Until the uploads that showed both are done in clientb's channel too.
    await_together(&agents.talk, &all, start);
    let since = agents.talk.cloud().relay.mark();
    let ended = now();

    drop(claude_session);

    let after = await_in_each(&mut agents.talk, acme, "with claude offline", |body| {
        shows(body, &claude, AgentState::Offline)
    });
    let shown = now();
    for (body, earlier) in after.iter().zip(&before) {
        assert_between(listed(body, &claude).since, (ended, shown), "offline");
        let codex_now = listed(body, &codex);
        assert_eq!(codex_now.state, AgentState::Available, "{body:?}");
        assert_eq!(
            codex_now.since,
            listed(earlier, &codex).since,
            "codex's since moved: {body:?}"
        );
    }
    await_together(&agents.talk, &all, since);
    let other = ada_s_body(&agents.talk, clientb).expect("a value in clientb");
    assert!(
        other.agents.is_empty(),
        "clientb lists acme's agents: {other:?}"
    );
}

/// The person's `skald recv` of Ada's machine starts a helper; no agent of the device has run. Her
/// value in each channel of `acme` lists no agent.
#[test]
fn helper_of_a_device_with_no_agent_uploads_empty_agents() {
    let mut agents = Agents::new(
        [4_731_003_001, 4_731_003_002],
        ["ada-runs-no-agent", "bob-sees-an-empty-list"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);

    helped.received();

    let acme = acme_channels(&agents);
    let bodies = await_in_each(&mut agents.talk, &acme, "uploaded", |_| true);
    for body in bodies {
        assert!(body.agents.is_empty(), "{body:?}");
    }
}
