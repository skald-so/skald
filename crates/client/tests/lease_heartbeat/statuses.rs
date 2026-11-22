//! The agent's state while it holds a claim: `busy` with `started`, `waiting` with
//! `awaiting-approval`, and `available` again once it holds none.

use std::thread;
use std::time::{Duration, Instant};

use skald_proto::{AgentState, PresenceBody};

use crate::claiming::{COMPLETE, Claims};
use crate::cloud::WAIT;
use crate::fixtures::cert_of;
use crate::helped::Helped;
use crate::scene::{
    claude_enqueues, codex_awaits_approval, helped_claims, helped_codex_session, helped_runs, scene,
};
use crate::senders::LAPTOP_AGENT;
use crate::values::{shows, values_in};
use crate::world::now;

/// What the test enqueues and answers; no text is anywhere else.
const REVIEW: &str = "heartbeat test: claude asks for a review codex waits on";
const REVIEW_DONE: &str = "heartbeat test: codex finished the review";

/// Waits for up to [`WAIT`] until the second machine's value in the talk's channel lists codex in
/// `state`, Bob's device taking in its mailbox whenever the value is of an epoch it holds no key
/// of; fails unless one does.
#[track_caller]
fn await_codex(claims: &mut Claims, state: AgentState) {
    let codex = claims.codex();
    let laptop = claims.laptop().device;
    let signing = cert_of(claims.cloud(), claims.laptop()).body().signing;
    let at = claims.at;
    let deadline = Instant::now() + WAIT;
    let mut last: Option<PresenceBody> = None;
    loop {
        let talk = &mut claims.senders.agents.talk;
        let value = values_in(talk, at)
            .into_iter()
            .find(|value| value.device == laptop);
        if let Some(value) = value {
            if (talk.played[0].mls)
                .history_key(value.channel, value.epoch)
                .is_none()
            {
                talk.played[0].take_in(&talk.world.cloud);
            }
            if let Some(key) = talk.played[0].mls.history_key(value.channel, value.epoch) {
                let body = (key.presence_key().open(&value, &signing, now()))
                    .unwrap_or_else(|err| panic!("the second machine's value is dropped: {err}"));
                if shows(&body, &codex, state) {
                    return;
                }
                last = Some(body);
            }
        }
        assert!(
            Instant::now() < deadline,
            "codex not {state:?} within {WAIT:?}; the last value: {last:?}"
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// Codex's MCP session runs through the helper: the second machine's value lists codex
/// `available`. Codex claims claude's task: `busy`. Its `skald extend --progress
/// awaiting-approval`: `waiting`. Its complete: `available` again.
#[test]
fn agent_shows_busy_then_waiting_then_available() {
    let mut claims = scene(
        [4_269_041_001, 4_269_041_002],
        ["ada-shows-codex-busy", "bob-sees-codex-wait"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    await_codex(&mut claims, AgentState::Available);
    let task = claude_enqueues(&claims, REVIEW);

    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), task);
    await_codex(&mut claims, AgentState::Busy);

    codex_awaits_approval(&claims, &helped, task);
    await_codex(&mut claims, AgentState::Waiting);

    let id = task.to_string();
    let args = [id.as_str(), REVIEW_DONE];
    let ran = helped_runs(&claims, &helped, Some(LAPTOP_AGENT), (COMPLETE, &args));
    assert!(ran.success, "the complete: {ran:?}");
    await_codex(&mut claims, AgentState::Available);
}
