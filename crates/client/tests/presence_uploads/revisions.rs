//! `revision`: one more per value in each channel, and never reused after a killed run or helper.

use skald_proto::{ChannelId, SpaceId};

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::fixtures::kill_once;
use crate::helped::Helped;
use crate::kit::RECV;
use crate::talk::Talk;
use crate::values::{
    AIDER, GEMINI, acme_channels, ada_s_body, await_available, await_in_each, claude_code_session,
    helpers_beside, ran_as, status_in, uploads,
};
use crate::world::AGENT;

/// The flag of `skald recv` that waits for the first message.
const WAIT_FLAG: &str = "--wait";

/// The revision of Ada's value in each of `ats`, in order; fails unless the server holds one in
/// each.
#[track_caller]
fn revisions(talk: &Talk, ats: &[(SpaceId, ChannelId)]) -> Vec<u64> {
    (ats.iter())
        .map(|at| match ada_s_body(talk, *at) {
            Some(body) => body.revision,
            None => panic!("no value of Ada's in {}", at.1),
        })
        .collect()
}

/// Claude, codex and gemini each run `skald recv` one-shot in `~/work/acme`, in turn, each run
/// registering its agent. Each run uploads one value in each channel of `acme`, whose revision is
/// one more than the one before in that channel.
#[test]
fn revision_rises_by_one_per_value() {
    let agents = Agents::new(
        [4_731_031_001, 4_731_031_002],
        ["ada-counts-her-values", "bob-checks-the-count"],
    );
    let acme = acme_channels(&agents);
    let mut before: Option<Vec<u64>> = None;
    for agent in [CLAUDE, CODEX, GEMINI] {
        let since = agents.talk.cloud().relay.mark();

        ran_as(&agents, agent, &[RECV]);

        for at in acme {
            assert_eq!(uploads(&agents.talk, at, since), 1, "{agent} in {}", at.1);
        }
        let now = revisions(&agents.talk, &acme);
        if let Some(before) = &before {
            let next: Vec<u64> = before.iter().map(|revision| revision + 1).collect();
            assert_eq!(now, next, "the revisions after {agent}'s run");
        }
        before = Some(now);
    }
}

/// Claude runs `skald recv` one-shot in `~/work/acme`. Codex's `skald recv --wait`, one-shot,
/// which waits as nothing comes, is killed once Ada's value in the talk's channel lists codex.
/// Gemini's run then uploads in each channel of `acme` a value whose revision is above the one the
/// server held there.
#[test]
fn revision_rises_past_a_killed_run() {
    let agents = Agents::new(
        [4_731_032_001, 4_731_032_002],
        ["ada-killed-mid-upload", "bob-sees-no-reuse"],
    );
    let acme = acme_channels(&agents);
    ran_as(&agents, CLAUDE, &[RECV]);
    let codex = agents.address(CODEX);
    let run = (agents.talk.ada.machine).start_in(&agents.folder, &[AGENT, CODEX, RECV, WAIT_FLAG]);
    kill_once(run, "value listing codex", || {
        ada_s_body(&agents.talk, acme[1]).is_some_and(|body| status_in(&body, &codex).is_some())
    });
    let held = revisions(&agents.talk, &acme);

    ran_as(&agents, GEMINI, &[RECV]);

    let gemini = agents.address(GEMINI);
    for (at, held) in acme.iter().zip(held) {
        let body = ada_s_body(&agents.talk, *at).expect("a value of Ada's");
        assert!(status_in(&body, &gemini).is_some(), "{body:?}");
        assert!(body.revision > held, "revision {held} reused: {body:?}");
    }
}

/// Through a helper, claude runs an MCP session of `claude-code` in `~/work/acme`, shown
/// `available`. The helper is killed, and the session ends. Aider's `skald recv` of the machine's
/// copy starts a new helper; Ada's value in each channel of `acme` then lists aider, with a
/// revision above the one the server held there.
#[test]
fn revision_rises_past_a_killed_helper() {
    let mut agents = Agents::new(
        [4_731_033_001, 4_731_033_002],
        ["ada-loses-her-helper", "bob-sees-the-count-go-on"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let session = claude_code_session(&helped, &agents, CLAUDE);
    let (_, acme) = await_available(&mut agents, CLAUDE);
    let held = revisions(&agents.talk, &acme);
    for pid in helpers_beside(&helped, &[&session]) {
        helped.kill(pid);
    }
    drop(session);

    let ran = helped.run_in(&agents.folder, &[AGENT, AIDER, RECV]);

    assert!(ran.success, "{ran:?}");
    let aider = agents.address(AIDER);
    let after = await_in_each(&mut agents.talk, &acme, "listing aider", |body| {
        status_in(body, &aider).is_some()
    });
    for (body, held) in after.iter().zip(held) {
        assert!(body.revision > held, "revision {held} reused: {body:?}");
    }
}
