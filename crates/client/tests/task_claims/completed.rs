//! A task that passes the claim checks: extended at once with `started`, handed to the agent as
//! one line with its `#<ref>`, sender and level, extended with the agent's progress, and completed;
//! its result goes to the channel under `mls`, once, with a history entry.

use skald_proto::{MessageId, Outcome};

use crate::claiming::{
    AWAITING_APPROVAL, CLAIMED, COMPLETE, Claims, DONE, EXTEND, JSON_FLAG, NO_TASK, PROGRESS_FLAG,
    STARTED,
};
use crate::entries::entries_of;
use crate::fixtures::CHANNEL;
use crate::lines::{json_objects, refers_to};
use crate::senders::{OWN, labelled_of};
use crate::talk::ADA_HOST;

/// What the tests enqueue and answer; no text is anywhere else.
const BUILD_NOTES: &str = "claim test: claude asks for the release notes of build 412";
const NOTES_DONE: &str = "claim test: codex wrote the release notes of build 412";
const DIGEST: &str = "claim test: claude asks for a digest of the open incidents";

/// As claude on Ada's first machine, `skald enqueue` puts a task in the channel's `default`. As
/// codex on her second machine, `skald claim --channel <channel>` shows it as one line,
/// `[<ada>/claude-… · ada-workstation · own] #<ref>: <text>`, whose reference is the task's. The
/// server gives it `claimed` by the second machine's device, with the progress `started`. Codex's
/// `skald complete #<ref> <text>` makes it `done`. Bob's device reads one result of it in the
/// channel, under `mls` from the second machine's device: from codex, to claude, `hops` 1, `done`,
/// naming the claim and generation the server gives, with codex's text as its response. The
/// second machine uploaded that message's history entry.
#[test]
fn agent_claims_an_own_task_runs_it_started_and_its_result_reaches_the_channel_under_mls() {
    let mut claims = Claims::new(
        [4_267_011_001, 4_267_011_002],
        ["ada-claims-build-notes", "bob-reads-the-notes-result"],
    );
    let record = claims.claude_enqueues(BUILD_NOTES);
    let task = record.task;
    let claude_short = format!("{}/{}", claims.ada().handle, crate::agents::CLAUDE);

    let claimed = claims.codex_claimed(&[]);

    let line = labelled_of(&claimed.shown(), BUILD_NOTES, BUILD_NOTES);
    assert_eq!(
        (
            line.from.as_str(),
            line.device.as_str(),
            line.level.as_str()
        ),
        (claude_short.as_str(), ADA_HOST, OWN),
        "{line:?}"
    );
    let as_message = MessageId::from_bytes(*task.as_bytes());
    assert!(
        refers_to(&line.reference, as_message),
        "not the task's reference: {line:?}"
    );
    let read = claims.read(task);
    assert_eq!(read.summary(), (CLAIMED, Some(STARTED), 1), "{read:?}");
    let laptop = claims.laptop().device;
    assert_eq!(read.held().device, laptop, "{read:?}");
    let uploads_before = claims.talk().front.uploads().len();

    let reference = format!("#{}", line.reference);
    claims.codex_ran(COMPLETE, &[&reference, NOTES_DONE]);

    let done = claims.read(task);
    assert_eq!(done.state, DONE, "{done:?}");
    let claude = claims.claude();
    let one = claims.assert_codex_result(
        (task, read.held()),
        (&claude, 1),
        (Outcome::Done, NOTES_DONE),
    );
    let talk = claims.talk();
    let entries = entries_of(&talk.front, talk.channel, uploads_before, one.msg_id);
    assert!(
        entries.iter().any(|entry| entry.forwarded),
        "no history entry of the result uploaded: {entries:?}"
    );
}

/// Codex claims claude's task with `skald claim --channel <channel> --json`: one JSON object with
/// the task's ID, a reference to it, `from` claude, `level` `own`, the text, and the claim ID,
/// generation 1 and attempt 1 the server gives. `skald extend --progress awaiting-approval <task>`
/// makes the server give the progress `awaiting-approval`; `skald extend <task>`, `started`.
#[test]
fn agent_extends_its_claim_with_its_progress_and_json_has_every_field() {
    let claims = Claims::new(
        [4_267_012_001, 4_267_012_002],
        ["ada-extends-the-digest", "bob-sees-the-digest"],
    );
    let record = claims.claude_enqueues(DIGEST);
    let task = record.task;

    let claimed = claims.codex_claimed(&[JSON_FLAG]);

    let objects = json_objects(&claimed.stdout);
    let [object] = objects.as_slice() else {
        panic!("not one JSON object: {claimed:?}");
    };
    let read = claims.read(task);
    let held = read.held();
    let as_message = MessageId::from_bytes(*task.as_bytes());
    let field = |name: &str| object[name].as_str().unwrap_or_default().to_owned();
    assert_eq!(field("task"), task.to_string(), "{object}");
    assert!(
        refers_to(field("ref").trim_start_matches('#'), as_message),
        "{object}"
    );
    assert_eq!(field("from"), claims.claude(), "{object}");
    assert_eq!(field("level"), OWN, "{object}");
    assert_eq!(field("text"), DIGEST, "{object}");
    assert_eq!(field("claim_id"), held.claim_id.to_string(), "{object}");
    assert_eq!(
        (object["generation"].as_u64(), object["attempt"].as_u64()),
        (Some(held.generation), Some(1)),
        "{object}"
    );
    let id = task.to_string();

    claims.codex_ran(EXTEND, &[PROGRESS_FLAG, AWAITING_APPROVAL, &id]);

    let waiting = claims.read(task);
    assert_eq!(
        waiting.summary(),
        (CLAIMED, Some(AWAITING_APPROVAL), 1),
        "{waiting:?}"
    );

    claims.codex_ran(EXTEND, &[&id]);

    let running = claims.read(task);
    assert_eq!(
        running.summary(),
        (CLAIMED, Some(STARTED), 1),
        "{running:?}"
    );
}

/// With no task in the queue, the person's `skald claim --channel <channel>` on the second machine
/// succeeds and prints one line, saying `no task`.
#[test]
fn claim_with_no_task_says_so_in_one_line() {
    let claims = Claims::new(
        [4_267_013_001, 4_267_013_002],
        ["ada-finds-no-task", "bob-enqueues-nothing"],
    );

    let ran = claims.person_runs(crate::claiming::CLAIM, &[CHANNEL, &claims.channel]);

    assert!(ran.success, "{ran:?}");
    let shown = ran.shown();
    let lines: Vec<&str> = shown
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let [line] = lines.as_slice() else {
        panic!("not one line: {ran:?}");
    };
    assert!(line.to_lowercase().contains(NO_TASK), "{line:?}");
}
