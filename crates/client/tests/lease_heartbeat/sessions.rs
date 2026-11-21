//! Sessions through the helper: the helper keeps a claim of the session's agent while the session
//! runs, each third of the lease and at once on `awaiting-approval`; once the session is killed,
//! the lease ends; a claim the heartbeat finds lost, the agent's next tool result says so.

use std::thread;
use std::time::{Duration, Instant};

use crate::claiming::{AWAITING_APPROVAL, COMPLETE, DONE, LOST, QUEUED, STARTED, names_task, says};
use crate::helped::Helped;
use crate::queuing::run_in;
use crate::scene::{
    LEASE, THIRD, assert_extended, claude_enqueues, codex_awaits_approval, helped_claims,
    helped_codex_session, helped_runs, lease_ends, scene, stays_claimed,
};
use crate::senders::LAPTOP_AGENT;
use crate::{agents, claiming};

/// What the tests enqueue and answer; no text is anywhere else.
const SLOW: &str = "heartbeat test: claude asks for a slow migration codex keeps";
const SLOW_DONE: &str = "heartbeat test: codex finished the slow migration";
const KILLED: &str = "heartbeat test: claude asks for an index rebuild whose session dies";
const KILLED_DONE: &str = "heartbeat test: codex rebuilt the index after its session died";
const CANCELLED: &str = "heartbeat test: claude asks for a cleanup ada cancels";
const CANCELLED_DONE: &str = "heartbeat test: codex finished the cancelled cleanup";

/// How long a test waits for the agent's tool result to say its claim is lost.
const SAID_WITHIN: Duration = Duration::from_secs(20);

/// Codex's MCP session runs through the helper; codex claims claude's task through it. For three
/// leases the server gives the task `claimed` with `started`, an extend coming at least each
/// third of the lease. Codex's `skald extend --progress awaiting-approval` makes the server give
/// `awaiting-approval` at once, and for a lease more the task stays claimed with it. Codex's
/// complete then succeeds, the server gives the task `done`, and Bob's device reads its result.
#[test]
fn helper_keeps_a_claim_of_a_running_session_s_agent_for_three_leases() {
    let mut claims = scene(
        [4_269_011_001, 4_269_011_002],
        ["ada-runs-a-slow-migration", "bob-reads-the-migration"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let _session = helped_codex_session(&claims, &helped);
    let task = claude_enqueues(&claims, SLOW);
    let since = claims.cloud().relay.mark();
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), task);

    stays_claimed(&claims, task, STARTED, LEASE * 3);

    assert_extended(&claims, (since, Instant::now()), task, 7);
    codex_awaits_approval(&claims, &helped, task);
    let read = claims.read(task);
    assert_eq!(
        read.progress.as_deref(),
        Some(AWAITING_APPROVAL),
        "{read:?}"
    );
    let waiting = claims.cloud().relay.mark();
    stays_claimed(&claims, task, AWAITING_APPROVAL, LEASE + THIRD);
    assert_extended(&claims, (waiting, Instant::now()), task, 3);
    let id = task.to_string();
    let args = [id.as_str(), SLOW_DONE];
    let ran = helped_runs(&claims, &helped, Some(LAPTOP_AGENT), (COMPLETE, &args));
    assert!(ran.success, "the complete: {ran:?}");
    assert_eq!(claims.read(task).state, DONE);
    let got = claims.results_once_read(task);
    assert_eq!(got.len(), 1, "{got:?}");
}

/// Codex's MCP session runs through the helper and codex claims claude's task; the task stays
/// claimed for a lease. The session is killed: the server's reads show the lease end, and claude,
/// on Ada's first machine, claims the task. Codex's complete fails, saying `lost`, and Bob's
/// device reads no result.
#[test]
fn killed_session_lets_the_lease_end_and_its_agent_s_complete_gets_claim_lost() {
    let mut claims = scene(
        [4_269_012_001, 4_269_012_002],
        ["ada-loses-a-session", "bob-gets-no-index-result"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let session = helped_codex_session(&claims, &helped);
    let task = claude_enqueues(&claims, KILLED);
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), task);
    stays_claimed(&claims, task, STARTED, LEASE + THIRD);

    drop(session);

    let read = lease_ends(&claims, task);
    assert_eq!(read.state, QUEUED, "{read:?}");
    claiming::tampers(claims.cloud(), task, "not_before = NULL");
    let claude = (claims.senders.agents.folder.as_path(), Some(agents::CLAUDE));
    let channel = claims.channel.clone();
    let ran = run_in(
        &claims.ada().machine,
        claude,
        claiming::CLAIM,
        &[crate::fixtures::CHANNEL, &channel],
    );
    assert!(ran.success, "claude's claim: {ran:?}");
    assert_eq!(claims.read(task).held().device, claims.ada().device);
    let id = task.to_string();
    let args = [id.as_str(), KILLED_DONE];
    let ran = helped_runs(&claims, &helped, Some(LAPTOP_AGENT), (COMPLETE, &args));
    assert!(!ran.success, "a complete after the lease: {ran:?}");
    assert!(says(&ran.shown(), LOST), "{ran:?}");
    claims.assert_no_result(task, "a claim whose session was killed");
}

/// Codex's MCP session runs through the helper and codex claims claude's task; Ada's first device
/// cancels it through the API. The heartbeat's next extend gets `claim-lost`: one of codex's next
/// `inbox` results says its claim is lost. Codex's complete then fails, saying `lost`, and Bob's
/// device reads no result.
#[test]
fn claim_the_heartbeat_finds_lost_is_said_in_the_agent_s_next_tool_result() {
    let mut claims = scene(
        [4_269_013_001, 4_269_013_002],
        ["ada-cancels-a-cleanup", "bob-gets-no-cleanup-result"],
    );
    let helped = Helped::new(&claims.laptop().machine);
    let mut session = helped_codex_session(&claims, &helped);
    let task = claude_enqueues(&claims, CANCELLED);
    helped_claims(&claims, &helped, Some(LAPTOP_AGENT), task);
    claims.ada_cancels(task);

    let deadline = Instant::now() + SAID_WITHIN;
    let said = loop {
        let inbox = session.inbox();
        if says(&inbox, LOST) {
            break inbox;
        }
        assert!(
            Instant::now() < deadline,
            "no result said the claim is lost: {inbox:?}"
        );
        thread::sleep(Duration::from_millis(500));
    };

    assert!(names_task(&said, task), "{said:?}");
    let id = task.to_string();
    let args = [id.as_str(), CANCELLED_DONE];
    let ran = helped_runs(&claims, &helped, Some(LAPTOP_AGENT), (COMPLETE, &args));
    assert!(!ran.success, "a complete of a lost claim: {ran:?}");
    assert!(says(&ran.shown(), LOST), "{ran:?}");
    claims.assert_no_result(task, "a claim the heartbeat found lost");
}
