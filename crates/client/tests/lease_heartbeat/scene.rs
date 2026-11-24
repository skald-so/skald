//! What the tests share: the claims scene with the queue's lease shortened, codex's MCP sessions
//! and runs on the second machine, through a helper or one-shot, the extends the relay saw, and
//! waits on what the server gives of a task.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use skald_proto::TaskId;

use crate::claiming::{
    AWAITING_APPROVAL, CLAIM, CLAIMED, Claims, EXTEND, PROGRESS_FLAG, Read, names_task,
};
use crate::client::{CLIENT, Session, session_in};
use crate::fixtures::CHANNEL;
use crate::helped::Helped;
use crate::machine::Finished;
use crate::queuing::{as_agent, enqueued, referred, tasks};
use crate::senders::LAPTOP_AGENT;
use crate::values::helped_session;
use crate::world::AGENT;

/// The queue's lease in these tests, shortened in the server's database from its shortest, 30
/// seconds, and a third of it.
pub const LEASE: Duration = Duration::from_secs(6);
pub const THIRD: Duration = Duration::from_secs(2);

/// The longest gap between two extends of a claim the heartbeat keeps: a third of the lease, a
/// second more as the device counts whole seconds, and a second for a loaded machine.
pub const GAP: Duration = Duration::from_secs(4);

/// How long the tests wait for reads to show a lease's end: the server shows it within 10 seconds
/// of its end.
pub const LEASE_ENDS_WITHIN: Duration = Duration::from_secs(30);

/// How often the tests read a task while they wait.
const READ_EVERY: Duration = Duration::from_millis(500);

/// The claims scene of the tests of claims (`task_claims/main.rs`), the lease of its queue
/// `default` shortened to [`LEASE`] past the database's check of the shortest.
#[track_caller]
pub fn scene(ids: [u64; 2], logins: [&str; 2]) -> Claims {
    Claims::with_lease(ids, logins, LEASE.as_secs())
}

/// Starts codex's `skald --agent codex-v5d8tm mcp` of `helped`'s copy, the second machine's, in its
/// `~/work/acme`, which starts a helper.
pub fn helped_codex_session(claims: &Claims, helped: &Helped) -> Session {
    let folder = claims.senders.laptop_folder.as_path();
    helped_session(helped, folder, &[], (LAPTOP_AGENT, CLIENT))
}

/// Starts codex's `skald --agent codex-v5d8tm mcp` one-shot on the second machine in its
/// `~/work/acme`.
pub fn one_shot_codex_session(claims: &Claims) -> Session {
    let folder = claims.senders.laptop_folder.as_path();
    session_in(
        &claims.laptop().machine,
        folder,
        &[AGENT, LAPTOP_AGENT],
        CLIENT,
    )
}

/// Runs `skald [--agent <agent>] <command> <args>` of `helped`'s copy on the second machine in its
/// `~/work/acme`, through its helper.
pub fn helped_runs(
    claims: &Claims,
    helped: &Helped,
    agent: Option<&str>,
    (command, args): (&str, &[&str]),
) -> Finished {
    helped.run_in(laptop_folder(claims), &as_agent(agent, command, args))
}

/// As [`helped_runs`] of codex's `extend --progress awaiting-approval <task>`, which must succeed.
#[track_caller]
pub fn codex_awaits_approval(claims: &Claims, helped: &Helped, task: TaskId) {
    let id = task.to_string();
    let args = [PROGRESS_FLAG, AWAITING_APPROVAL, id.as_str()];
    let ran = helped_runs(claims, helped, Some(LAPTOP_AGENT), (EXTEND, &args));
    assert!(ran.success, "the extend: {ran:?}");
}

/// As [`helped_runs`] of `claim --channel <channel>`; fails unless it succeeds and names `task`.
#[track_caller]
pub fn helped_claims(claims: &Claims, helped: &Helped, agent: Option<&str>, task: TaskId) {
    let args = [CHANNEL, claims.channel.as_str()];
    let ran = helped_runs(claims, helped, agent, (CLAIM, &args));
    assert!(ran.success, "the claim: {ran:?}");
    assert!(
        names_task(&ran.shown(), task),
        "{task} not claimed: {ran:?}"
    );
}

/// As claude on the first machine, `skald enqueue --channel <channel> <text>`, which must succeed;
/// gives the task it enqueued.
#[track_caller]
pub fn claude_enqueues(claims: &Claims, text: &str) -> TaskId {
    let claude = (
        claims.senders.agents.folder.as_path(),
        Some(crate::agents::CLAUDE),
    );
    let ran = enqueued(
        &claims.ada().machine,
        claude,
        &[CHANNEL, &claims.channel, text],
    );
    let held = tasks(claims.cloud(), claims.ada(), claims.at, claims.queue);
    referred(&held, &ran).task.task
}

/// When each extend of `task` the machines sent from `since` on came to the relay, in order.
pub fn extends_since(claims: &Claims, since: Instant, task: TaskId) -> Vec<Instant> {
    let named = format!("/tasks/{task}/extend ");
    let mut came: Vec<Instant> = (claims.cloud().relay.sent_since(since).into_iter())
        .filter(|sent| (sent.head.lines().next()).is_some_and(|line| line.contains(&named)))
        .map(|sent| sent.at)
        .collect();
    came.sort();
    came
}

/// Fails unless each extend of `task` from `since` to `until` came within [`GAP`] of the one
/// before, the first within it of `since`, the last within it of `until`, and at least `least`
/// came.
#[track_caller]
pub fn assert_extended(
    claims: &Claims,
    (since, until): (Instant, Instant),
    task: TaskId,
    least: usize,
) {
    let came: Vec<Instant> = (extends_since(claims, since, task).into_iter())
        .filter(|at| *at <= until)
        .collect();
    assert!(
        came.len() >= least,
        "{} extends of {task} in {:?}, not {least}",
        came.len(),
        until - since
    );
    let edges: Vec<Instant> = [since].into_iter().chain(came).chain([until]).collect();
    for pair in edges.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap <= GAP, "{gap:?} without an extend of {task}");
    }
}

/// Reads `task` every half second for `span`; fails unless each read gives it `claimed` with
/// `progress`. Gives the last read.
#[track_caller]
pub fn stays_claimed(claims: &Claims, task: TaskId, progress: &str, span: Duration) -> Read {
    let until = Instant::now() + span;
    loop {
        let read = claims.read(task);
        assert_eq!(read.state, CLAIMED, "{task} lost its claim: {read:?}");
        assert_eq!(read.progress.as_deref(), Some(progress), "{read:?}");
        if Instant::now() >= until {
            return read;
        }
        thread::sleep(READ_EVERY);
    }
}

/// Reads `task` every half second until it is no longer `claimed`; fails unless that comes within
/// [`LEASE_ENDS_WITHIN`]. Gives that read.
#[track_caller]
pub fn lease_ends(claims: &Claims, task: TaskId) -> Read {
    let deadline = Instant::now() + LEASE_ENDS_WITHIN;
    loop {
        let read = claims.read(task);
        if read.state != CLAIMED {
            return read;
        }
        assert!(
            Instant::now() < deadline,
            "the lease of {task} has not ended: {read:?}"
        );
        thread::sleep(READ_EVERY);
    }
}

/// The folder of codex's runs on the second machine.
pub fn laptop_folder(claims: &Claims) -> &Path {
    claims.senders.laptop_folder.as_path()
}
