//! An agent's `skald tasks` lists only the tasks it enqueued, naming no ID, while the person's lists
//! every task; an agent is refused a restart and a cancel, which change nothing.

use crate::agents::{CLAUDE, CODEX};
use crate::lines::{JSON_FLAG, assert_no_uuid, json_objects};
use crate::queuing::{DEFAULT, enqueued, referred, run_in, tasks};
use crate::states::{
    CANCEL_TASK, DEAD, FAILED, QUEUED, RESTART_TASK, Scene, assert_listed, assert_not_listed,
    claims, fails, listed, read, reference_of,
};

/// What claude, codex and the person enqueue; no text is anywhere else.
const CLAUDE_S: &str = "own test: claude asks for the flaky test to be fixed";
const CODEX_S: &str = "own test: codex asks for the docs to be built";
const PERSON_S: &str = "own test: ada asks for the release notes";

/// In `~/work/acme`, Ada's machine puts a task as claude, one as codex and one as the person; Bob's
/// device claims claude's, the first, and fails it without retry. As claude, `skald tasks --channel
/// <channel>` lists claude's task `dead` with `failed` in `default` with attempts 1, neither of the
/// others, and no ID; with `--json` it gives claude's task alone. The person's `skald tasks
/// --channel <channel>` lists all three. As claude, `skald restart-task --channel <channel>
/// #<ref>` and `skald cancel-task --channel <channel> #<ref>` of its task, and `skald cancel-task
/// --channel <channel> #<ref>` of the person's, each fail; no request restarts or cancels a task,
/// and the tasks stay as they were.
#[test]
fn agent_lists_only_its_own_tasks_and_is_refused_restarts_and_cancels() {
    let scene = Scene::new(
        [7_402_006_001, 7_402_006_002],
        ["ada-runs-two-agents", "bob-fails-claude-s-task"],
    );
    scene.agents.run_as(CODEX);
    let (cloud, ada, bob, at, queue) = scene.parts();
    let person = scene.as_person();
    let (claude, codex) = (scene.as_agent(CLAUDE), scene.as_agent(CODEX));
    let in_channel = scene.in_channel(&[]);
    let puts = |by, text: &str| {
        let ran = enqueued(scene.machine(), by, &scene.in_channel(&[text]));
        let task = referred(&tasks(cloud, ada, at, queue), &ran).task.task;
        (task, reference_of(&ran, task))
    };
    let (claude_s, claude_ref) = puts(claude, CLAUDE_S);
    let (codex_s, _) = puts(codex, CODEX_S);
    let (person_s, person_ref) = puts(person, PERSON_S);
    let held = claims(cloud, bob, at, queue);
    assert_eq!(held.task, claude_s, "Bob's device claimed another task");
    fails(cloud, bob, &held, false);

    let shown = listed(scene.machine(), claude, &in_channel);

    assert_listed(&shown, claude_s, &[DEAD, FAILED, DEFAULT, "1"], &[QUEUED]);
    assert_not_listed(&shown, codex_s);
    assert_not_listed(&shown, person_s);
    assert_no_uuid(&shown.shown());

    let json = listed(scene.machine(), claude, &scene.in_channel(&[JSON_FLAG]));

    let objects = json_objects(&json.stdout);
    let ids: Vec<&str> = (objects.iter())
        .filter_map(|object| object["task"].as_str())
        .collect();
    assert_eq!(
        (ids, objects.len()),
        (vec![claude_s.to_string().as_str()], 1),
        "not claude's task alone: {json:?}"
    );

    let all = listed(scene.machine(), person, &in_channel);

    assert_listed(&all, claude_s, &[DEAD, FAILED], &[]);
    assert_listed(&all, codex_s, &[QUEUED], &[]);
    assert_listed(&all, person_s, &[QUEUED], &[]);

    let since = cloud.relay.mark();
    let refusals = [
        (RESTART_TASK, claude_ref.as_str()),
        (CANCEL_TASK, claude_ref.as_str()),
        (CANCEL_TASK, person_ref.as_str()),
    ];
    for (command, reference) in refusals {
        let refused = run_in(
            scene.machine(),
            claude,
            command,
            &scene.in_channel(&[reference]),
        );
        assert!(
            !refused.success,
            "claude's {command} {reference}: {refused:?}"
        );
    }

    let writes: Vec<String> = (cloud.relay.sent_since(since).into_iter())
        .filter_map(|sent| sent.head.lines().next().map(str::to_lowercase))
        .filter(|line| {
            let target = line.split(' ').nth(1).unwrap_or_default();
            target.ends_with("/restart") || target.ends_with("/cancel")
        })
        .collect();
    assert!(
        writes.is_empty(),
        "a task restarted or cancelled as claude: {writes:?}"
    );
    let claude_now = read(cloud, ada, (at, queue), claude_s);
    assert_eq!(claude_now.shows(), (DEAD, 1, None, Some(FAILED)));
    assert_eq!(read(cloud, ada, (at, queue), person_s).state, QUEUED);
}
