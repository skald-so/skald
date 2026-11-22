//! A result from the claiming device reaches the agent that enqueued the task once, and the
//! claiming device gets `read`: across two machines, and on one.

use crate::agents::{CODEX, run_as_in};
use crate::claiming::{CLAIM, COMPLETE};
use crate::client::{CLIENT, session_in};
use crate::fixtures::CHANNEL;
use crate::queuing::run_in;
use crate::receipts::shows_read;
use crate::returns::{DONE, OWN, Returns, assert_no_result, assert_one_result};
use crate::senders::LAPTOP_AGENT;
use crate::world::AGENT;

/// What the tests enqueue and answer; no text is anywhere else.
const BUILD_NOTES: &str = "result test: claude asks for the release notes of build 977";
const NOTES_DONE: &str = "result test: codex wrote the release notes of build 977";
const DIGEST: &str = "result test: claude asks its neighbour codex for an incident digest";
const DIGEST_DONE: &str = "result test: codex sums up the open incidents";

/// As claude on Ada's first machine, `skald enqueue` puts a task in the channel's `default`; as
/// codex on her second machine, `skald claim` and `skald complete` run it. Bob's device reads the
/// result in the channel. claude's next `skald recv` shows one line of that result: the task's
/// reference, `task done`, codex and `own`; the run after it shows it no more. The person's
/// `skald recv` on the second machine shows `read` of the result.
#[test]
fn the_enqueuing_agent_gets_the_claimer_s_done_result_once_and_the_claimer_gets_read() {
    let mut returns = Returns::new(
        [5_873_001_001, 5_873_001_002],
        ["ada-gets-the-notes-result", "bob-watches-the-notes-result"],
    );
    let task = returns.claude_enqueues(BUILD_NOTES);
    returns.claims.codex_claimed(&[]);
    returns
        .claims
        .codex_ran(COMPLETE, &[&task.to_string(), NOTES_DONE]);
    let sent = returns.claims.results_once_read(task);
    let [result] = sent.as_slice() else {
        panic!("not one result sent: {sent:?}");
    };

    let got = returns.claude_reads();

    assert_one_result(&got.shown(), task, DONE, (LAPTOP_AGENT, OWN));
    let again = returns.claude_reads();
    assert_no_result(&again.shown(), task, "claude's second run");
    shows_read(&returns.claims.laptop().machine, result.msg_id);
}

/// On Ada's first machine alone: claude enqueues a task, and codex, another agent of the same
/// machine in the same folder, claims and completes it. The `inbox` tool of claude's `skald mcp`
/// shows one line of the result: the task's reference, `task done`, codex and `own`. The person's
/// `skald recv` on that machine shows `read` of the result, which Bob's device reads in the
/// channel.
#[test]
fn on_one_device_the_enqueuing_agent_gets_the_result_in_its_inbox_and_the_claimer_gets_read() {
    let mut returns = Returns::new(
        [5_873_002_001, 5_873_002_002],
        ["ada-runs-both-agents", "bob-sees-one-machine-results"],
    );
    let task = returns.claude_enqueues(DIGEST);
    let agents = &returns.claims.senders.agents;
    let (machine, folder) = (&agents.talk.ada.machine, agents.folder.as_path());
    run_as_in(machine, folder, CODEX);
    let channel = returns.channel().to_string();
    let claimed = run_in(machine, (folder, Some(CODEX)), CLAIM, &[CHANNEL, &channel]);
    assert!(claimed.success, "codex's claim: {claimed:?}");
    assert!(claimed.shown().contains(DIGEST), "{claimed:?}");
    let completed = run_in(
        machine,
        (folder, Some(CODEX)),
        COMPLETE,
        &[&task.to_string(), DIGEST_DONE],
    );
    assert!(completed.success, "codex's complete: {completed:?}");
    let sent = returns.claims.results_once_read(task);
    let [result] = sent.as_slice() else {
        panic!("not one result sent: {sent:?}");
    };
    let agents = &returns.claims.senders.agents;
    let (machine, folder) = (&agents.talk.ada.machine, agents.folder.as_path());

    let mut session = session_in(machine, folder, &[AGENT, crate::agents::CLAUDE], CLIENT);
    let inbox = session.inbox();
    drop(session);

    assert_one_result(&inbox, task, DONE, (CODEX, OWN));
    shows_read(machine, result.msg_id);
}
