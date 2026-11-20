//! Values under the epoch the device enters, and none under an epoch it judged unclean
//! (`docs/protocol/membership.md`, "Clean epochs").

use skald_proto::{AgentState, Role, Roster};

use crate::agents::{Agents, CLAUDE, CODEX};
use crate::chain::{bob_commits_empty, carol};
use crate::entries::killed_in_unclean_epoch;
use crate::helped::Helped;
use crate::kit::{RECV, rosters};
use crate::talk::Talk;
use crate::values::{ada_s_value, await_value, claude_code_session, ran_as, shows, status_in};
use crate::world::{ACME, serve_revisions};

/// Through a helper, claude runs an MCP session of `claude-code` in `~/work/acme`, and Ada's value
/// in the talk's channel lists it `available`. Bob's device commits an empty Commit there,
/// starting the next epoch: Ada's helper, with nothing else changed, uploads a value under it,
/// which Bob's key of that epoch opens, listing claude `available`.
#[test]
fn helper_uploads_under_the_new_epoch_on_entering_it() {
    let mut agents = Agents::new(
        [4_731_051_001, 4_731_051_002],
        ["ada-follows-the-epochs", "bob-commits-an-epoch"],
    );
    let helped = Helped::new(&agents.talk.ada.machine);
    let _session = claude_code_session(&helped, &agents, CLAUDE);
    let claude = agents.address(CLAUDE);
    let at = agents.talk.at();
    let (value, _) = await_value(&mut agents.talk, at, "with claude available", |_, body| {
        shows(body, &claude, AgentState::Available)
    });
    let next = value.epoch + 1;

    bob_commits_empty(&mut agents.talk, next);

    await_value(
        &mut agents.talk,
        at,
        "under the next epoch",
        |value, body| {
            value.epoch == next && body.epoch == next && shows(body, &claude, AgentState::Available)
        },
    );
}

/// As the test of history entries in an unclean epoch (`history_entries/unclean.rs`): the relay
/// keeps the later revisions of `acme` from Ada's machine; Bob's device, an admin's, adds Carol to
/// the talk's channel, and her device to its group, epoch 2; Ada's run is killed once it entered
/// epoch 2 and judged it unclean, before it commits Carol's Remove; then the revisions go through.
/// Claude's `skald recv`, one-shot in `~/work/acme`, registers claude: Ada's value in the default
/// channel lists it, and none of hers in the talk's channel is under epoch 2. Bob's device then
/// commits an empty Commit, epoch 3, which Ada's machine judges clean: codex's run uploads a value
/// there under epoch 3, listing claude and codex.
#[test]
fn nothing_is_sealed_under_an_epoch_judged_unclean() {
    let talk = Talk::with(
        ACME,
        &[4_731_052_001, 4_731_052_002],
        &["ada-in-an-unclean-epoch", "bob-adds-carol-unseen"],
        false,
        (Role::Admin, |_| {}),
    );
    let mut agents = Agents::bound(talk);
    let space = agents.talk.space.id;
    let seen: Vec<Roster> = rosters(agents.talk.cloud(), &agents.talk.ada.token, space);
    let served: Vec<&Roster> = seen.iter().collect();
    serve_revisions(agents.talk.cloud(), space, &served);
    let carol = carol(&agents.talk, 4_731_052_003, "carol-admitted-unseen");
    killed_in_unclean_epoch(&mut agents.talk, &carol);
    let (default, at) = ((space, agents.talk.space.default), agents.talk.at());
    let claude = agents.address(CLAUDE);

    ran_as(&agents, CLAUDE, &[RECV]);

    await_value(&mut agents.talk, default, "listing claude", |_, body| {
        status_in(body, &claude).is_some()
    });
    if let Some(value) = ada_s_value(&agents.talk, at) {
        assert_ne!(value.epoch, 2, "a value sealed under epoch 2: {value:?}");
    }

    bob_commits_empty(&mut agents.talk, 3);
    ran_as(&agents, CODEX, &[RECV]);

    let codex = agents.address(CODEX);
    await_value(&mut agents.talk, at, "under epoch 3", |value, body| {
        value.epoch == 3 && status_in(body, &claude).is_some() && status_in(body, &codex).is_some()
    });
}
