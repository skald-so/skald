//! A complete kept in the local state before its request: a CLI killed after the server took it
//! repeats it at its next run and sends the result once.

use skald_proto::Outcome;

use crate::claiming::{COMPLETE, Claims, DONE, agent_reads, complete_request};
use crate::fixtures::kill_once;
use crate::relay::Lose;
use crate::senders::LAPTOP_AGENT;

/// What the test enqueues and answers; no text is anywhere else.
const REPORT: &str = "crash test: claude asks for the weekly uptime report";
const REPORT_DONE: &str = "crash test: codex sent the weekly uptime report";

/// Codex claims claude's task. The relay withholds the server's answer to its complete, and the
/// test kills `skald complete <task> <text>` once that answer is lost; the server gives the task
/// `done`. Codex's next `skald recv` on the second machine, and the one after it, leave Bob's
/// device with one result of the task, `done`, with codex's text.
#[test]
fn cli_killed_after_its_complete_sends_the_result_once_at_its_next_run() {
    let mut claims = Claims::new(
        [4_267_041_001, 4_267_041_002],
        ["ada-kills-the-report", "bob-reads-one-report"],
    );
    let task = claims.claude_enqueues(REPORT).task;
    claims.codex_claimed(&[]);
    let held = claims.read(task).held().clone();
    let cloud = claims.cloud();
    cloud.relay.lose_answer(
        &complete_request(claims.at, claims.queue, task),
        Lose::Withhold,
    );
    let id = task.to_string();
    let args = ["--agent", LAPTOP_AGENT, COMPLETE, &id, REPORT_DONE];
    let started = (claims.laptop().machine).start_in(&claims.senders.laptop_folder, &args);

    kill_once(started, "a lost answer to the complete", || {
        cloud.relay.lost() > 0
    });

    assert_eq!(claims.read(task).state, DONE, "the complete not taken");
    let (machine, folder) = (&claims.laptop().machine, &claims.senders.laptop_folder);
    agent_reads(machine, folder, LAPTOP_AGENT);
    agent_reads(machine, folder, LAPTOP_AGENT);
    let claude = claims.claude();
    claims.assert_codex_result((task, &held), (&claude, 1), (Outcome::Done, REPORT_DONE));
}
