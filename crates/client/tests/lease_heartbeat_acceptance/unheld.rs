//! Claims with no session of their agent: a claim as the person, and one of an agent whose session
//! does not run, get no heartbeat beside a session that keeps its own agent's claim; each holds only
//! as long as `skald extend` keeps it.

use std::time::Instant;

use skald_proto::TaskId;

use crate::agents::{CLAUDE, run_as_in};
use crate::claiming::{CLAIM, COMPLETE, DONE, EXTEND, QUEUED, STARTED};
use crate::fixtures::CHANNEL;
use crate::queuing::{enqueued, run_in, tasks};
use crate::scene::{Scene, THIRD, assert_held, waits};
use crate::values::GEMINI;

/// What the tests enqueue and answer; no text is anywhere else.
const KEPT: &str = "heartbeat test: claude asks codex for a report its session keeps";
const KEPT_DONE: &str = "heartbeat test: codex finished the report its session kept";
const PERSON_S: &str = "heartbeat test: claude asks for a backup ada claims herself";
const GEMINI_S: &str = "heartbeat test: claude asks for a cleanup gemini claims without a session";

/// As claude on Ada's first machine, `skald enqueue --channel <channel> <text>`, which must
/// succeed; gives the task the queue `default` holds that it did not before.
#[track_caller]
fn claude_enqueues(scene: &Scene, text: &str) -> TaskId {
    let claims = &scene.claims;
    let before = tasks(claims.cloud(), claims.ada(), claims.at, claims.queue);
    let claude = (claims.senders.agents.folder.as_path(), Some(CLAUDE));
    let args = [CHANNEL, claims.channel.as_str(), text];
    enqueued(&claims.ada().machine, claude, &args);
    let after = tasks(claims.cloud(), claims.ada(), claims.at, claims.queue);
    let new: Vec<TaskId> = (after.iter().map(|stored| stored.task.task))
        .filter(|task| !before.iter().any(|stored| stored.task.task == *task))
        .collect();
    let [one] = new.as_slice() else {
        panic!("not one new task: {new:?}");
    };
    *one
}

/// The second machine runs through a helper, codex's `skald mcp` runs through it, and gemini has
/// run once in `~/work/acme` but runs no session. Claude enqueues three tasks: codex claims the
/// first, the person on the second machine the second, and gemini the third, each with `skald
/// claim`. Two thirds of the lease after its claim, gemini's `skald extend <task>` extends the
/// third once. The person's task gets no extend after its claim, and its lease ends while the
/// server's reads still show the third held, then the third's lease ends too, with no extend but
/// gemini's own. Throughout, codex's task stays `claimed` by its claim with `started`, extended
/// each third of the lease; codex's `skald complete <task> <text>` then succeeds.
#[test]
fn claims_of_the_person_and_of_an_agent_with_no_session_get_no_heartbeat_beside_one_kept() {
    let scene = Scene::new(
        [4_812_041_001, 4_812_041_002],
        ["ada-claims-a-backup-herself", "bob-watches-the-leases"],
    );
    let laptop = &scene.claims.laptop().machine;
    let folder = scene.claims.senders.laptop_folder.as_path();
    run_as_in(laptop, folder, GEMINI);
    let helped = scene.helped();
    let _session = scene.helped_session(&helped);
    let kept = claude_enqueues(&scene, KEPT);
    let person_s = claude_enqueues(&scene, PERSON_S);
    let gemini_s = claude_enqueues(&scene, GEMINI_S);
    let channel = scene.claims.channel.clone();
    let (kept_held, kept_since) = scene.codex_holds(kept);

    let claimed = scene.claims.person_runs(CLAIM, &[CHANNEL, &channel]);
    assert!(claimed.success, "the person's claim: {claimed:?}");
    let person_after = scene.claims.cloud().relay.mark();
    let person_held = scene.held_by_laptop(person_s);
    let gemini_started = Instant::now();
    let claimed = run_in(laptop, (folder, Some(GEMINI)), CLAIM, &[CHANNEL, &channel]);
    assert!(claimed.success, "gemini's claim: {claimed:?}");
    let gemini_after = scene.claims.cloud().relay.mark();
    let gemini_held = scene.held_by_laptop(gemini_s);
    waits((THIRD * 2).saturating_sub(gemini_started.elapsed()));
    let gemini_extends = scene.claims.cloud().relay.mark();
    let id = gemini_s.to_string();
    let extended = run_in(laptop, (folder, Some(GEMINI)), EXTEND, &[&id]);
    assert!(extended.success, "gemini's extend: {extended:?}");
    let gemini_extended = scene.claims.cloud().relay.mark();

    let person_ended = scene.await_lease_end(person_s, &person_held);

    assert_eq!(person_ended.state, QUEUED, "{person_ended:?}");
    assert_held(&scene.claims.read(gemini_s), &gemini_held, STARTED);
    scene.assert_no_extend(person_after, person_s, "the person's claim");
    let gemini_ended = scene.await_lease_end(gemini_s, &gemini_held);
    assert_eq!(gemini_ended.state, QUEUED, "{gemini_ended:?}");
    let before_its_extend: Vec<Instant> = (scene.extends_since(gemini_after, gemini_s).into_iter())
        .filter(|at| *at < gemini_extends)
        .collect();
    assert!(
        before_its_extend.is_empty(),
        "the claim of an agent with no session extended before its own extend"
    );
    scene.assert_no_extend(gemini_extended, gemini_s, "after gemini's extend");
    assert_held(&scene.claims.read(kept), &kept_held, STARTED);
    scene.assert_heartbeat(kept_since, kept);
    scene
        .claims
        .codex_ran(COMPLETE, &[&kept.to_string(), KEPT_DONE]);
    assert_eq!(scene.claims.read(kept).state, DONE);
}
